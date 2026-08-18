use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use tracing::warn;

use crate::connection::{token_matches, AuthContext};

/// Reads stay public (game clients and bots fetch them); writes require the
/// local NPC token or a Google sign-in from an allowlisted admin email.
pub async fn require_admin_for_writes(
    State(auth): State<Arc<AuthContext>>,
    req: Request,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(next.run(req).await);
    }

    let token = bearer_token(req.headers()).ok_or_else(|| {
        warn!("REST write rejected: missing bearer token");
        unauthorized()
    })?;

    if token_matches(token, &auth.npc_token) {
        return Ok(next.run(req).await);
    }

    let Some(verifier) = &auth.google else {
        warn!("REST write rejected: no Google verifier configured");
        return Err(unauthorized());
    };
    let claims = verifier.verify(token).await.map_err(|e| {
        warn!("REST write rejected: {e}");
        unauthorized()
    })?;

    if !auth.is_admin(&claims) {
        warn!(
            "REST write rejected: {} is not an admin",
            claims.email.as_deref().unwrap_or("<no email>")
        );
        return Err((StatusCode::FORBIDDEN, "not an admin".to_string()));
    }
    Ok(next.run(req).await)
}

/// The bearer credential on a request, whichever kind it is — the admin
/// allowlist checks one here, the cape upload checks a player's session token.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn unauthorized() -> (StatusCode, String) {
    (StatusCode::UNAUTHORIZED, "unauthorized".to_string())
}
