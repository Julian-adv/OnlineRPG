//! REST surface for player cape textures: one authenticated upload, one
//! public fetch, and an admin block. Mounted outside
//! `require_admin_for_writes` — the upload checks a player's own session
//! token instead of the admin allowlist.

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_TYPE},
        HeaderMap, StatusCode,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::sync::Arc;
use tracing::warn;

use crate::api_auth;
use crate::cape_texture::{CapeTextureStore, UploadError, MAX_UPLOAD_BYTES};
use crate::connection::AuthContext;

#[derive(Serialize)]
struct UploadResponse {
    hash: String,
}

pub fn cape_texture_router(store: Arc<CapeTextureStore>, auth: Arc<AuthContext>) -> Router {
    let admin = Router::new()
        .route("/api/cape-texture/{hash}/block", post(block_texture))
        .layer(axum::middleware::from_fn_with_state(
            auth,
            api_auth::require_admin_for_writes,
        ))
        .with_state(Arc::clone(&store));

    Router::new()
        .route(
            "/api/cape-texture",
            post(upload_texture).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/api/cape-texture/{hash}", get(get_texture))
        .with_state(store)
        .merge(admin)
}

/// Raw PNG body, authenticated with the upload token the server handed the
/// connection at login. Answers with the stored image's content hash, which
/// the client then names in `ApplyCapeTexture`.
async fn upload_texture(
    State(store): State<Arc<CapeTextureStore>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<UploadResponse>, (StatusCode, String)> {
    let token = api_auth::bearer_token(&headers).unwrap_or_default();

    match store.store(token, &body).await {
        Ok(hash) => Ok(Json(UploadResponse { hash })),
        Err(err) => {
            let status = match err {
                UploadError::NotSignedIn => StatusCode::UNAUTHORIZED,
                UploadError::BadHash => StatusCode::BAD_REQUEST,
                UploadError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
                UploadError::NotAnImage => StatusCode::BAD_REQUEST,
                UploadError::Blocked => StatusCode::FORBIDDEN,
                UploadError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
                UploadError::Io(ref e) => {
                    warn!("Cape texture upload failed: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            };
            Err((status, err.to_string()))
        }
    }
}

/// Public and immutable: the name is the content, so a cached copy can never
/// be stale. Nearby players fetch this once per texture, whatever the crowd.
async fn get_texture(
    State(store): State<Arc<CapeTextureStore>>,
    Path(hash): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    // No stat first: the read is the existence check, and this is the one
    // route every nearby client hits.
    if !store.may_serve(&hash).await {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = tokio::fs::read(store.path(&hash))
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok((
        [
            (CONTENT_TYPE, "image/png"),
            (CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    ))
}

async fn block_texture(
    State(store): State<Arc<CapeTextureStore>>,
    Path(hash): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    store
        .block(&hash)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cape_texture::MAX_TEXTURE_SIZE;

    fn png(size: u32) -> Vec<u8> {
        crate::test_util::test_png(size, [10, 120, 200, 255])
    }

    /// The router as `main` mounts it: the admin middleware wraps everything
    /// merged before it, and the cape routes are merged after.
    async fn serve() -> (String, Arc<CapeTextureStore>) {
        let store = Arc::new(
            CapeTextureStore::new(crate::test_util::unique_temp_dir("cape_routes")).expect("store"),
        );
        let auth = Arc::new(AuthContext {
            google: None,
            npc_token: "npc-token".to_string(),
            admin_emails: Vec::new(),
        });
        let app = Router::new()
            .layer(axum::middleware::from_fn_with_state(
                Arc::clone(&auth),
                api_auth::require_admin_for_writes,
            ))
            .merge(cape_texture_router(Arc::clone(&store), auth));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (base, store)
    }

    /// The whole player-facing round trip: upload with the session token, then
    /// fetch what came back. A regression here is a cape that never prints.
    #[tokio::test]
    async fn a_signed_in_player_uploads_and_the_world_can_fetch_it() {
        let (base, store) = serve().await;
        let token = store.open_session("account").await;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{base}/api/cape-texture"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "image/png")
            .body(png(64))
            .send()
            .await
            .expect("upload");
        assert_eq!(response.status(), 200, "the admin gate must not catch this");
        let hash = response.json::<serde_json::Value>().await.expect("json")["hash"]
            .as_str()
            .expect("hash")
            .to_string();

        let fetched = client
            .get(format!("{base}/api/cape-texture/{hash}"))
            .send()
            .await
            .expect("fetch");
        assert_eq!(fetched.status(), 200);
        assert_eq!(
            fetched
                .headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("public, max-age=31536000, immutable"),
            "the name is the content, so the copy can never go stale"
        );
        assert!(!fetched.bytes().await.expect("body").is_empty());

        store.block(&hash).await.expect("blocked");
        let gone = client
            .get(format!("{base}/api/cape-texture/{hash}"))
            .send()
            .await
            .expect("fetch");
        assert_eq!(gone.status(), 404, "a blocked print stops being served");
    }

    #[tokio::test]
    async fn an_unsigned_upload_and_an_oversized_one_are_both_refused() {
        let (base, store) = serve().await;
        let client = reqwest::Client::new();

        let no_token = client
            .post(format!("{base}/api/cape-texture"))
            .body(png(64))
            .send()
            .await
            .expect("upload");
        assert_eq!(no_token.status(), 401);

        let token = store.open_session("account").await;
        let too_big = client
            .post(format!("{base}/api/cape-texture"))
            .header("Authorization", format!("Bearer {token}"))
            .body(png(MAX_TEXTURE_SIZE + 8))
            .send()
            .await
            .expect("upload");
        assert_eq!(too_big.status(), 413);
    }

    /// Blocking stays behind the admin allowlist the rest of the REST API
    /// uses; only the upload steps around it.
    #[tokio::test]
    async fn blocking_still_needs_an_admin() {
        let (base, _store) = serve().await;
        let hash = "0".repeat(64);

        let refused = reqwest::Client::new()
            .post(format!("{base}/api/cape-texture/{hash}/block"))
            .send()
            .await
            .expect("block");
        assert_eq!(refused.status(), 401);

        let allowed = reqwest::Client::new()
            .post(format!("{base}/api/cape-texture/{hash}/block"))
            .header("Authorization", "Bearer npc-token")
            .send()
            .await
            .expect("block");
        assert_eq!(allowed.status(), 204);
    }
}
