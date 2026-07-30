//! Server-side sampler for the unified water surface (WFD1 — sea + rivers),
//! the twin of `HeightSampler`. `depth = surfaceY − bedHeight > 0` is true
//! over ocean AND rivers and false on land (`doc/WATER_SYSTEM.md`).
//!
//! Sea-only tiles have no baked file (the client synthesizes a flat sea field
//! for them); a missing tile therefore samples as `SEA_LEVEL`.

use onlinerpg_shared::worldgen::tile_bake::{
    WATER_FIELD_BIN_MAGIC, WATER_FIELD_HEADER_SIZE, WATER_FIELD_PIXEL_SIZE, WATER_FIELD_TOTAL_SIZE,
};

use crate::coords::world_to_tile;
use crate::defaults::VERTS_PER_SIDE;
use crate::height::{bilinear, decode_height, resolve_cell};
use crate::io::TerrainIO;
use crate::tile_cache::{TileCache, TileCacheReadGuard, TILE_CACHE_CAPACITY};

/// Fixed ocean surface used where a tile has no baked water field. Mirrors
/// the client's sea-field synthesis and the bake's `SEA_LEVEL_M`.
pub const SEA_LEVEL: f32 = 0.0;

/// Surface value at a tile-local vertex; tiles without a baked field (`None`)
/// read as `SEA_LEVEL`, matching the client's flat-sea synthesis.
fn get_surface_at_cell(
    cache: &TileCacheReadGuard<'_, DecodedWaterTile>,
    tx: i32,
    tz: i32,
    cell_x: i32,
    cell_z: i32,
) -> f32 {
    let (key, idx) = resolve_cell(tx, tz, cell_x, cell_z);
    match cache.get(&key) {
        Some(Some(surfaces)) => surfaces.get(idx).copied().unwrap_or(SEA_LEVEL),
        _ => SEA_LEVEL,
    }
}

/// Where the raw WFD1 tiles come from — the local data directory on the game
/// server, the same source `HeightTiles` reads. `None` means the tile has no
/// baked water field (sea-only / not generated).
#[async_trait::async_trait]
pub trait WaterTiles: Send + Sync {
    async fn read_water_field(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>>;
}

#[async_trait::async_trait]
impl WaterTiles for TerrainIO {
    async fn read_water_field(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>> {
        TerrainIO::read_water_field(self, tx, tz).await
    }
}

/// None = tile has no water field (sea-only). Some = decoded surfaceY grid.
type DecodedWaterTile = Option<Vec<f32>>;

/// Samples the baked water surface with an in-memory per-tile cache. Interior
/// mutability (like `HeightSampler`) so callers hold only `&self`.
pub struct WaterSampler {
    cache: TileCache<DecodedWaterTile>,
    tiles: Box<dyn WaterTiles>,
}

impl WaterSampler {
    pub fn new(tiles: impl WaterTiles + 'static) -> Self {
        Self {
            cache: TileCache::new(TILE_CACHE_CAPACITY),
            tiles: Box::new(tiles),
        }
    }

    /// Decode a WFD1 buffer's surfaceY channel into a 65×65 grid. Returns
    /// `None` (treated as sea-only) if the bytes are malformed, so a corrupt
    /// tile degrades to open sea rather than erroring the cast.
    fn decode_surfaces(raw: &[u8]) -> Option<Vec<f32>> {
        if raw.len() != WATER_FIELD_TOTAL_SIZE || &raw[0..4] != WATER_FIELD_BIN_MAGIC {
            return None;
        }
        let mut surfaces = Vec::with_capacity(VERTS_PER_SIDE * VERTS_PER_SIDE);
        let mut offset = WATER_FIELD_HEADER_SIZE;
        for _ in 0..(VERTS_PER_SIDE * VERTS_PER_SIDE) {
            let v = u16::from_le_bytes([raw[offset], raw[offset + 1]]);
            surfaces.push(decode_height(v));
            offset += WATER_FIELD_PIXEL_SIZE;
        }
        Some(surfaces)
    }

    async fn ensure_tile(&self, tx: i32, tz: i32) -> std::io::Result<()> {
        if self.cache.contains(&(tx, tz)).await {
            return Ok(());
        }
        let decoded = self
            .tiles
            .read_water_field(tx, tz)
            .await?
            .and_then(|raw| Self::decode_surfaces(&raw));
        self.cache.insert_if_absent((tx, tz), decoded).await;
        Ok(())
    }

    /// Water surface height at a world position, bilinearly interpolated.
    /// `SEA_LEVEL` where no water field is baked. Loads tiles on demand.
    pub async fn sample_surface(&self, world_x: f32, world_z: f32) -> std::io::Result<f32> {
        self.ensure_tile(world_to_tile(world_x), world_to_tile(world_z))
            .await?;
        let cache = self.cache.read().await;
        Ok(bilinear(world_x, world_z, |tx, tz, cx, cz| {
            get_surface_at_cell(&cache, tx, tz, cx, cz)
        }))
    }

    /// Evict tiles not sampled since the previous call; run on a period so
    /// the cache tracks the live working set instead of growing forever.
    pub async fn sweep_stale_tiles(&self) -> usize {
        self.cache.sweep_stale().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_shared::worldgen::tile_bake::{HEIGHT_BIAS, HEIGHT_STEP};

    /// Build a WFD1 buffer whose every pixel carries `surface_m`.
    fn wfd1_uniform(surface_m: f32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(WATER_FIELD_BIN_MAGIC);
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(VERTS_PER_SIDE as u16).to_le_bytes());
        out.extend_from_slice(&(VERTS_PER_SIDE as u16).to_le_bytes());
        out.extend_from_slice(&[0u8; 6]);
        let enc = ((surface_m + HEIGHT_BIAS) / HEIGHT_STEP).round() as u16;
        for _ in 0..(VERTS_PER_SIDE * VERTS_PER_SIDE) {
            out.extend_from_slice(&enc.to_le_bytes());
            out.push(0); // flowX
            out.push(0); // flowZ
            out.push(255); // riverness
            out.push(0); // turbulence
        }
        out
    }

    /// A river tile at surfaceY = `surface_m`; every other tile has no field.
    struct OneRiverTile {
        river_tile: (i32, i32),
        surface_m: f32,
    }

    #[async_trait::async_trait]
    impl WaterTiles for OneRiverTile {
        async fn read_water_field(&self, tx: i32, tz: i32) -> std::io::Result<Option<Vec<u8>>> {
            if (tx, tz) == self.river_tile {
                Ok(Some(wfd1_uniform(self.surface_m)))
            } else {
                Ok(None)
            }
        }
    }

    #[tokio::test]
    async fn missing_tile_samples_as_sea_level() {
        let sampler = WaterSampler::new(OneRiverTile {
            river_tile: (999, 999),
            surface_m: 42.0,
        });
        // A world point in a tile with no field reads as flat sea.
        let s = sampler.sample_surface(0.0, 0.0).await.unwrap();
        assert!((s - SEA_LEVEL).abs() < 1e-3, "expected sea level, got {s}");
    }

    #[tokio::test]
    async fn river_tile_samples_its_surface() {
        // River channel surface at +5.4 m — well above sea level, the case the
        // old height<0 check wrongly rejected.
        let river_tile = (world_to_tile(200.0), world_to_tile(300.0));
        let sampler = WaterSampler::new(OneRiverTile {
            river_tile,
            surface_m: 5.4,
        });
        let s = sampler.sample_surface(200.0, 300.0).await.unwrap();
        assert!((s - 5.4).abs() < 0.05, "expected ~5.4 m, got {s}");
    }
}
