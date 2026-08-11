//! Splatmap tiles for the agent: what the ground is made of (road, sand,
//! cliff, river bed...), sampled per world position. Mirrors the heightmap
//! plumbing — the same `/api/terrain/splat/{tx}/{tz}` endpoint the web
//! client's ground shader reads, or the local terrain directory when the
//! agent runs on the game server.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use onlinerpg_terrain::coords::world_to_tile;
use onlinerpg_terrain::defaults::{SPLATMAP_SIZE, TILE_DIM};
use onlinerpg_terrain::io::TerrainIO;

/// Splat palette indices, from the baker that writes them.
pub use onlinerpg_shared::worldgen::tile_bake::{
    PAL_CLIFF, PAL_PAVING, PAL_RIVER_BED, PAL_ROAD, PAL_SAND, PAL_SNOW, PAL_STONE_PATH,
};

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

/// Splat tiles over HTTP with a disk cache — the same plumbing as the
/// heightmap twin, via the shared `HttpTiles` source.
pub struct HttpSplatTiles(crate::terrain_http::HttpTiles);

impl HttpSplatTiles {
    pub fn new(base_url: &str, cache_dir: PathBuf) -> Self {
        Self(crate::terrain_http::HttpTiles::new(
            base_url,
            cache_dir,
            "splat",
            "s_",
            SPLATMAP_SIZE,
        ))
    }
}

#[async_trait::async_trait]
impl SplatTiles for HttpSplatTiles {
    async fn read_splat(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        match self.0.read(tx, tz).await? {
            Some(data) => Ok(data),
            // The splat endpoint answers unbaked tiles with a default 200,
            // so a 404 means the URL is wrong — surface it.
            None => Err(std::io::Error::other(format!(
                "splat tile {tx},{tz}: unexpected 404"
            ))),
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
    /// cell's first byte). Tiles span [t*DIM - DIM/2, t*DIM + DIM/2) at 1m
    /// per cell.
    pub async fn primary_at(&self, world_x: f32, world_z: f32) -> std::io::Result<u8> {
        let (tx, tz) = (world_to_tile(world_x), world_to_tile(world_z));
        let tile = self.tile(tx, tz).await?;
        let dim = TILE_DIM as i32;
        let size = TILE_DIM as f32;
        let cell_x = ((world_x - (tx as f32 * size - size / 2.0)).floor() as i32).clamp(0, dim - 1);
        let cell_z = ((world_z - (tz as f32 * size - size / 2.0)).floor() as i32).clamp(0, dim - 1);
        let idx = (cell_z * dim + cell_x) as usize * 4;
        Ok(tile.get(idx).map_or(0, |b| b >> 4))
    }
}
