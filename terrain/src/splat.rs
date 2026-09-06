//! Server-side sampler for the splatmap (what the ground is made of), the
//! twin of `HeightSampler`. Ambient spawns land on the vegetation base slot
//! only — not on road, sand, cliff, river bed or snow (`doc/SPLATMAP_V2.md`).

use crate::coords::world_to_tile;
use crate::defaults::TILE_DIM;
use crate::io::TerrainIO;
use crate::tile_cache::{TileCache, TILE_CACHE_CAPACITY};

/// Palette slot holding the vegetation-supporting base texture — the baker's
/// own constant, mirrored client-side as `VEGETATION_BASE_SLOT`.
pub use onlinerpg_shared::worldgen::tile_bake::PAL_GROUND as VEGETATION_BASE_SLOT;

/// Where raw splat tiles come from: the terrain directory on the game server,
/// or the public tile API for clients running elsewhere.
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

/// Samples the baked splatmap with the shared per-tile cache. Cells are
/// decoded to their dominant palette slot on the way in: a quarter of the raw
/// bytes stay resident and the blend branch leaves the sample path.
pub struct SplatSampler {
    cache: TileCache<Vec<u8>>,
    tiles: Box<dyn SplatTiles>,
    revision: tokio::sync::RwLock<u64>,
}

impl SplatSampler {
    pub fn new(tiles: impl SplatTiles + 'static) -> Self {
        Self {
            cache: TileCache::new(TILE_CACHE_CAPACITY),
            tiles: Box::new(tiles),
            revision: tokio::sync::RwLock::new(0),
        }
    }

    /// One slot per cell: the blend picks which of the cell's two palette
    /// indices is actually shown, as the ground shader does.
    fn decode_dominant(raw: &[u8]) -> Vec<u8> {
        raw.as_chunks::<4>()
            .0
            .iter()
            .map(|cell| {
                if cell[2] >= 128 {
                    cell[0] & 0x0f
                } else {
                    cell[0] >> 4
                }
            })
            .collect()
    }

    async fn ensure_tile(&self, tx: i32, tz: i32) -> std::io::Result<()> {
        loop {
            let before = *self.revision.read().await;
            if self.cache.contains(&(tx, tz)).await {
                return Ok(());
            }
            let raw = self.tiles.read_splat(tx, tz).await?;
            let revision = self.revision.read().await;
            if *revision != before {
                continue;
            }
            self.cache
                .insert_if_absent((tx, tz), Self::decode_dominant(&raw))
                .await;
            return Ok(());
        }
    }

    /// Dominant palette slot at a world position. Cells are 1m; tiles span
    /// `[t*DIM - DIM/2, t*DIM + DIM/2)`.
    pub async fn dominant_at(&self, world_x: f32, world_z: f32) -> std::io::Result<u8> {
        let (tx, tz) = (world_to_tile(world_x), world_to_tile(world_z));
        self.ensure_tile(tx, tz).await?;
        let dim = TILE_DIM as i32;
        let size = TILE_DIM as f32;
        let cell_x = ((world_x - (tx as f32 * size - size / 2.0)).floor() as i32).clamp(0, dim - 1);
        let cell_z = ((world_z - (tz as f32 * size - size / 2.0)).floor() as i32).clamp(0, dim - 1);
        let idx = (cell_z * dim + cell_x) as usize;
        let cache = self.cache.read().await;
        // The sweep can evict between the insert and this read; say so rather
        // than inventing a slot.
        cache
            .get(&(tx, tz))
            .and_then(|tile| tile.get(idx).copied())
            .ok_or_else(|| std::io::Error::other(format!("splat tile {tx},{tz} went missing")))
    }

    /// Is the ground here the vegetation base — grassland rather than road,
    /// sand, cliff, river bed or snow?
    pub async fn is_vegetation_base_at(&self, world_x: f32, world_z: f32) -> bool {
        self.dominant_at(world_x, world_z)
            .await
            .is_ok_and(|slot| slot == VEGETATION_BASE_SLOT)
    }

    pub async fn sweep_stale_tiles(&self) -> usize {
        self.cache.sweep_stale().await
    }

    pub async fn invalidate_tile(&self, tx: i32, tz: i32) {
        let mut revision = self.revision.write().await;
        *revision += 1;
        self.cache
            .remove(&(crate::coords::wrap_tile_x(tx), tz))
            .await;
    }

    pub async fn update_tile(&self, tx: i32, tz: i32, raw: &[u8]) {
        let mut revision = self.revision.write().await;
        *revision += 1;
        self.cache
            .replace(
                (crate::coords::wrap_tile_x(tx), tz),
                Self::decode_dominant(raw),
            )
            .await;
    }
}
