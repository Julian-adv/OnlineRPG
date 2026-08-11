//! Splatmap tiles for the agent: what the ground is made of (road, sand,
//! cliff, river bed...), sampled per world position. Mirrors the heightmap
//! plumbing — the same `/api/terrain/splat/{tx}/{tz}` endpoint the web
//! client's ground shader reads, or the local terrain directory when the
//! agent runs on the game server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use onlinerpg_terrain::coords::world_to_tile;
use onlinerpg_terrain::defaults::{SPLATMAP_SIZE, TILE_DIM};
use onlinerpg_terrain::io::TerrainIO;
use tracing::{debug, warn};

/// Splat palette indices (`shared/src/worldgen/tile_bake/constants.rs`).
pub const PAL_SAND: u8 = 1;
pub const PAL_SNOW: u8 = 3;
pub const PAL_ROAD: u8 = 4;
pub const PAL_CLIFF: u8 = 5;
pub const PAL_RIVER_BED: u8 = 6;
pub const PAL_STONE_PATH: u8 = 7;
pub const PAL_PAVING: u8 = 8;

/// Where raw splat tiles come from: the terrain directory or HTTP.
#[async_trait::async_trait]
pub trait SplatTiles: Send + Sync {
    async fn read_splat(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>>;
}

#[async_trait::async_trait]
impl SplatTiles for TerrainIO {
    async fn read_splat(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        self.read_splatmap(tx, tz).await
    }
}

/// Splat tiles over HTTP with a disk cache, the twin of `HttpHeightTiles`.
pub struct HttpSplatTiles {
    base_url: String,
    cache_dir: PathBuf,
    http: reqwest::Client,
}

impl HttpSplatTiles {
    pub fn new(base_url: &str, cache_dir: PathBuf) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            cache_dir,
            http: reqwest::Client::new(),
        }
    }

    fn cache_path(&self, tx: i32, tz: i32) -> PathBuf {
        self.cache_dir.join(format!("s_{tx}_{tz}.bin"))
    }

    async fn read_cached(path: &Path) -> Option<Vec<u8>> {
        match tokio::fs::read(path).await {
            Ok(data) if data.len() == SPLATMAP_SIZE => Some(data),
            Ok(data) => {
                warn!(
                    "Cached splatmap {:?} has wrong size {} — refetching",
                    path,
                    data.len()
                );
                None
            }
            Err(_) => None,
        }
    }

    async fn write_cached(path: &Path, data: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("part");
        tokio::fs::write(&tmp, data).await?;
        tokio::fs::rename(&tmp, path).await
    }

    async fn fetch(&self, tx: i32, tz: i32) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/api/terrain/splat/{tx}/{tz}", self.base_url);
        let response = self.http.get(&url).send().await?.error_for_status()?;
        let bytes = response.bytes().await?.to_vec();
        if bytes.len() != SPLATMAP_SIZE {
            anyhow::bail!(
                "{url} returned {} bytes, expected {SPLATMAP_SIZE}",
                bytes.len()
            );
        }
        Ok(bytes)
    }
}

#[async_trait::async_trait]
impl SplatTiles for HttpSplatTiles {
    async fn read_splat(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        let path = self.cache_path(tx, tz);
        if let Some(cached) = Self::read_cached(&path).await {
            return Ok(cached);
        }
        match self.fetch(tx, tz).await {
            Ok(data) => {
                if let Err(e) = Self::write_cached(&path, &data).await {
                    warn!("Failed to cache splatmap {tx},{tz}: {e}");
                }
                debug!("Fetched splatmap tile ({tx}, {tz})");
                Ok(data)
            }
            Err(e) => Err(std::io::Error::other(e)),
        }
    }
}

type SplatTileCache = tokio::sync::RwLock<HashMap<(i32, i32), Arc<Vec<u8>>>>;

/// Tile-cached splat sampling by world position.
pub struct SplatSampler {
    tiles: Box<dyn SplatTiles>,
    cache: SplatTileCache,
}

/// Keep this many decoded tiles (64 tiles = a 512m square around the agent);
/// past that the cache is dropped wholesale rather than tracking LRU order.
const SPLAT_CACHE_CAP: usize = 64;

impl SplatSampler {
    pub fn new(tiles: impl SplatTiles + 'static) -> Self {
        Self {
            tiles: Box::new(tiles),
            cache: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    async fn tile(&self, tx: i32, tz: i32) -> std::io::Result<Arc<Vec<u8>>> {
        if let Some(t) = self.cache.read().await.get(&(tx, tz)) {
            return Ok(Arc::clone(t));
        }
        let data = Arc::new(self.tiles.read_splat(tx, tz).await?);
        let mut cache = self.cache.write().await;
        if cache.len() >= SPLAT_CACHE_CAP {
            cache.clear();
        }
        cache.insert((tx, tz), Arc::clone(&data));
        Ok(data)
    }

    /// Primary surface palette index at a world position (high nibble of the
    /// cell's first byte). Tiles span [t*64-32, t*64+32) at 1m per cell.
    pub async fn primary_at(&self, world_x: f32, world_z: f32) -> std::io::Result<u8> {
        let (tx, tz) = (world_to_tile(world_x), world_to_tile(world_z));
        let tile = self.tile(tx, tz).await?;
        let dim = TILE_DIM as i32;
        let cell_x = ((world_x - (tx as f32 * 64.0 - 32.0)).floor() as i32).clamp(0, dim - 1);
        let cell_z = ((world_z - (tz as f32 * 64.0 - 32.0)).floor() as i32).clamp(0, dim - 1);
        let idx = (cell_z * dim + cell_x) as usize * 4;
        Ok(tile.get(idx).map_or(0, |b| b >> 4))
    }
}
