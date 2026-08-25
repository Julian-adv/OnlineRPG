use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use sha2::Digest;
use std::sync::Arc;
use tracing::{error, warn};

use super::io::TerrainIO;
use crate::game_state::GameState;

/// The objects route also feeds the game state's furniture passability,
/// so it carries both IO and game state.
#[derive(Clone)]
struct ObjectsState {
    terrain: Arc<TerrainIO>,
    game_state: Arc<GameState>,
}

#[derive(Deserialize)]
struct MinimapQuery {
    size: Option<u32>,
}

pub fn terrain_router(terrain_io: Arc<TerrainIO>, game_state: Arc<GameState>) -> Router {
    let objects_router = Router::new()
        .route(
            "/api/terrain/objects/{rx}/{rz}",
            get(get_object).put(put_object),
        )
        .with_state(ObjectsState {
            terrain: Arc::clone(&terrain_io),
            game_state,
        });
    Router::new()
        .route(
            "/api/terrain/height/{x}/{z}",
            get(get_heightmap).put(put_heightmap),
        )
        .route(
            "/api/terrain/splat/{x}/{z}",
            get(get_splatmap).put(put_splatmap),
        )
        .route(
            "/api/terrain/height-original/{x}/{z}",
            get(get_original_heightmap).put(put_original_heightmap),
        )
        .route(
            "/api/terrain/height-original/{x}/{z}/ensure",
            post(ensure_original_heightmap),
        )
        .route(
            "/api/terrain/grass/{x}/{z}",
            get(get_grass)
                .put(put_grass)
                .layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route(
            "/api/terrain/grass-original/{x}/{z}",
            get(get_original_grass)
                .put(put_original_grass)
                .layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route(
            "/api/terrain/grass-original/{x}/{z}/ensure",
            post(ensure_original_grass),
        )
        .route(
            "/api/terrain/minimap/{rx}/{rz}",
            get(get_minimap).put(put_minimap),
        )
        .route("/api/terrain/zones/{rx}/{rz}", get(get_zone).put(put_zone))
        .route("/api/terrain/trees/{x}/{z}", get(get_trees))
        .route("/api/terrain/river-field/{x}/{z}", get(get_river_field))
        .route("/api/terrain/water-field/{x}/{z}", get(get_water_field))
        .route(
            "/api/terrain/region/{rx}/{rz}",
            delete(delete_region_handler),
        )
        .with_state(terrain_io)
        .merge(objects_router)
}

async fn get_heightmap(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_heightmap(x, z).await.map_err(|e| {
        error!("Failed to read heightmap ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

async fn put_heightmap(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    terrain
        .write_heightmap(x, z, &body)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidData => (StatusCode::BAD_REQUEST, e.to_string()),
            _ => {
                error!("Failed to write heightmap ({}, {}): {}", x, z, e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_original_heightmap(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_original_heightmap(x, z).await.map_err(|e| {
        error!("Failed to read original heightmap ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match data {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn put_original_heightmap(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    terrain
        .write_original_heightmap(x, z, &body)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidData => (StatusCode::BAD_REQUEST, e.to_string()),
            _ => {
                error!("Failed to write original heightmap ({}, {}): {}", x, z, e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_original_grass(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_original_grass(x, z).await.map_err(|e| {
        error!("Failed to read original grass ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match data {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn put_original_grass(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    terrain
        .write_original_grass(x, z, &body)
        .await
        .map_err(|e| {
            error!("Failed to write original grass ({}, {}): {}", x, z, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ensure_original_heightmap(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<StatusCode, StatusCode> {
    let created = terrain.ensure_original_heightmap(x, z).await.map_err(|e| {
        error!("Failed to ensure original heightmap ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    if created {
        Ok(StatusCode::CREATED)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

async fn ensure_original_grass(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<StatusCode, StatusCode> {
    let created = terrain.ensure_original_grass(x, z).await.map_err(|e| {
        error!("Failed to ensure original grass ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    if created {
        Ok(StatusCode::CREATED)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

async fn get_splatmap(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_splatmap(x, z).await.map_err(|e| {
        error!("Failed to read splatmap ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

async fn put_splatmap(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    terrain
        .write_splatmap(x, z, &body)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidData => (StatusCode::BAD_REQUEST, e.to_string()),
            _ => {
                error!("Failed to write splatmap ({}, {}): {}", x, z, e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_grass(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_grass(x, z).await.map_err(|e| {
        error!("Failed to read grass ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match data {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn put_grass(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    terrain.write_grass(x, z, &body).await.map_err(|e| {
        error!("Failed to write grass ({}, {}): {}", x, z, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error".to_string(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_minimap(
    Path((rx, rz)): Path<(i32, i32)>,
    Query(query): Query<MinimapQuery>,
    request_headers: axum::http::HeaderMap,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let size = query.size.unwrap_or(1024);
    if ![128, 256, 512, 1024].contains(&size) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let found = terrain.stat_minimap_lod(rx, rz, size).await.map_err(|e| {
        error!("Failed to stat minimap ({}, {}): {}", rx, rz, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    // The client's ?v= only changes on client deploys, so a server-side rebake
    // must reach players through revalidation: cache briefly, then let the
    // ETag turn repeat fetches into bodyless 304s. The tag comes from the
    // file's identity (path/mtime/len) so revalidation never reads the body.
    // 404s cache briefly too, so clients near unbaked regions don't hammer
    // the route.
    const MINIMAP_CACHE: (header::HeaderName, &str) =
        (header::CACHE_CONTROL, "public, max-age=300");
    let Some((path, meta)) = found else {
        return Ok((StatusCode::NOT_FOUND, [MINIMAP_CACHE]).into_response());
    };
    let etag = minimap_etag(&path, &meta);
    let revalidated = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag);
    if revalidated {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, etag)],
            [MINIMAP_CACHE],
        )
            .into_response());
    }
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        error!("Failed to read minimap ({}, {}): {}", rx, rz, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let content_type = if bytes.starts_with(b"RIFF") {
        "image/webp"
    } else {
        "image/png"
    };
    Ok((
        [(header::ETAG, etag)],
        [(header::CONTENT_TYPE, content_type.to_string())],
        [MINIMAP_CACHE],
        bytes,
    )
        .into_response())
}

/// Cache tag for a minimap file: which file was picked, plus its mtime and
/// size. A rebake or an editor save changes all three sources cheaply.
fn minimap_etag(path: &std::path::Path, meta: &std::fs::Metadata) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(meta.len().to_le_bytes());
    if let Ok(modified) = meta.modified() {
        if let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) {
            hasher.update(since_epoch.as_nanos().to_le_bytes());
        }
    }
    let digest = hasher.finalize();
    let tag = digest
        .iter()
        .take(8)
        .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
    format!("\"{tag:016x}\"")
}

async fn put_minimap(
    Path((rx, rz)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    terrain.write_minimap(rx, rz, &body).await.map_err(|e| {
        error!("Failed to write minimap ({}, {}): {}", rx, rz, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_zone(
    Path((rx, rz)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let zone = terrain.read_zone(rx, rz).await.map_err(|e| {
        error!("Failed to read zone ({}, {}): {}", rx, rz, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(zone).into_response())
}

async fn put_zone(
    Path((rx, rz)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    terrain.write_zone(rx, rz, &body).await.map_err(|e| {
        error!("Failed to write zone ({}, {}): {}", rx, rz, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error".to_string(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_object(
    Path((rx, rz)): Path<(i32, i32)>,
    State(state): State<ObjectsState>,
) -> Result<Response, StatusCode> {
    let data = state.terrain.read_object(rx, rz).await.map_err(|e| {
        error!("Failed to read object ({}, {}): {}", rx, rz, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(data).into_response())
}

async fn put_object(
    Path((rx, rz)): Path<(i32, i32)>,
    State(state): State<ObjectsState>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    let placements = GameState::parse_region_furniture(&body).map_err(|e| {
        warn!("Invalid region objects ({rx},{rz}): {e}");
        (
            StatusCode::BAD_REQUEST,
            "Invalid region object data".to_string(),
        )
    })?;
    state
        .terrain
        .write_object(rx, rz, &body)
        .await
        .map_err(|e| {
            error!("Failed to write object ({}, {}): {}", rx, rz, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        })?;
    state.game_state.sync_region_furniture(rx, rz, &placements);
    Ok(StatusCode::NO_CONTENT)
}

async fn get_trees(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_trees(x, z).await.map_err(|e| {
        error!("Failed to read trees ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match data {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_river_field(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_river_field(x, z).await.map_err(|e| {
        error!("Failed to read river field ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match data {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_water_field(
    Path((x, z)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<Response, StatusCode> {
    let data = terrain.read_water_field(x, z).await.map_err(|e| {
        error!("Failed to read water field ({}, {}): {}", x, z, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match data {
        Some(bytes) => {
            Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn delete_region_handler(
    Path((rx, rz)): Path<(i32, i32)>,
    State(terrain): State<Arc<TerrainIO>>,
) -> Result<StatusCode, StatusCode> {
    terrain.delete_region(rx, rz).await.map_err(|e| {
        error!("Failed to delete region ({}, {}): {}", rx, rz, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(StatusCode::NO_CONTENT)
}
