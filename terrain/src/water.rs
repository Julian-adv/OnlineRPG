//! Server-side sampler for the unified water surface (WFD1 — sea + rivers),
//! the twin of `HeightSampler`. The water field bakes one `surfaceY` per
//! world point: sea level over open ocean, the carved river surface inside a
//! channel, and a smoothmax blend at estuaries; on land the surface collapses
//! below the terrain. So `depth = surfaceY − bedHeight > 0` is true over
//! ocean AND rivers and false on land — which is exactly the "is this water?"
//! test fishing needs (`doc/WATER_SYSTEM.md`, `doc/RIVER_SYSTEM.md`).
//!
//! Sea-only tiles have no baked file (the client synthesizes a flat sea field
//! for them); a missing tile therefore samples as `SEA_LEVEL`, matching that
//! synthesis so open ocean far from any river still reads as water.

use std::collections::HashMap;

use crate::coords::world_to_tile;
use crate::defaults::{self, VERTS_PER_SIDE};
use crate::io::TerrainIO;

/// Must match the heightmap tile size (client `TERRAIN_TILE_SIZE`).
const TILE_SIZE: f32 = defaults::TILE_DIM as f32;

/// Fixed ocean surface used where a tile has no baked water field. Mirrors
/// the client's sea-field synthesis and the bake's `SEA_LEVEL_M`.
pub const SEA_LEVEL: f32 = 0.0;

/// WFD1 layout (see `shared/src/worldgen/tile_bake/water_field.rs`):
/// 16-byte header then 65×65 pixels of 6 bytes each (u16 surfaceY, i8 flowX,
/// i8 flowZ, u8 riverness, u8 turbulence), row-major X then Z.
const WFD_HEADER_BYTES: usize = 16;
const WFD_PIXEL_BYTES: usize = 6;
const WFD_MAGIC: &[u8; 4] = b"WFD1";

/// Decode a surfaceY sample (same encoding as the heightmap: HEIGHT_STEP 0.05,
/// HEIGHT_BIAS 500).
fn decode_surface(value: u16) -> f32 {
    value as f32 * 0.05 - 500.0
}

fn tile_key(tx: i32, tz: i32) -> (i32, i32) {
    (tx, tz)
}

/// Surface value at a tile-local vertex, resolving cross-tile lookups. A tile
/// with no baked field (`None`) reads as `SEA_LEVEL` everywhere, matching the
/// client's flat-sea synthesis for sea-only tiles.
fn get_surface_at_cell(
    cache: &HashMap<(i32, i32), Option<Vec<f32>>>,
    tx: i32,
    tz: i32,
    cell_x: i32,
    cell_z: i32,
) -> f32 {
    let (mut tx, mut tz, mut cx, mut cz) = (tx, tz, cell_x, cell_z);

    if cx >= VERTS_PER_SIDE as i32 {
        tx += 1;
        cx -= defaults::TILE_DIM as i32;
    } else if cx < 0 {
        tx -= 1;
        cx += defaults::TILE_DIM as i32;
    }
    if cz >= VERTS_PER_SIDE as i32 {
        tz += 1;
        cz -= defaults::TILE_DIM as i32;
    } else if cz < 0 {
        tz -= 1;
        cz += defaults::TILE_DIM as i32;
    }

    match cache.get(&tile_key(tx, tz)) {
        Some(Some(surfaces)) => {
            let idx = cz as usize * VERTS_PER_SIDE + cx as usize;
            surfaces.get(idx).copied().unwrap_or(SEA_LEVEL)
        }
        // Tile with no water file, or not loaded: flat sea.
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

/// Samples the baked water surface with an in-memory per-tile cache. Interior
/// mutability (like `HeightSampler`) so callers hold only `&self`.
pub struct WaterSampler {
    // None = tile has no water field (sea-only). Some = decoded surfaceY grid.
    cache: tokio::sync::RwLock<HashMap<(i32, i32), Option<Vec<f32>>>>,
    tiles: Box<dyn WaterTiles>,
}

impl WaterSampler {
    pub fn new(tiles: impl WaterTiles + 'static) -> Self {
        Self {
            cache: tokio::sync::RwLock::new(HashMap::new()),
            tiles: Box::new(tiles),
        }
    }

    /// Decode a WFD1 buffer's surfaceY channel into a 65×65 grid. Returns
    /// `None` (treated as sea-only) if the bytes are malformed, so a corrupt
    /// tile degrades to open sea rather than erroring the cast.
    fn decode_surfaces(raw: &[u8]) -> Option<Vec<f32>> {
        let expected = WFD_HEADER_BYTES + VERTS_PER_SIDE * VERTS_PER_SIDE * WFD_PIXEL_BYTES;
        if raw.len() != expected || &raw[0..4] != WFD_MAGIC {
            return None;
        }
        let mut surfaces = Vec::with_capacity(VERTS_PER_SIDE * VERTS_PER_SIDE);
        let mut offset = WFD_HEADER_BYTES;
        for _ in 0..(VERTS_PER_SIDE * VERTS_PER_SIDE) {
            let v = u16::from_le_bytes([raw[offset], raw[offset + 1]]);
            surfaces.push(decode_surface(v));
            offset += WFD_PIXEL_BYTES;
        }
        Some(surfaces)
    }

    async fn ensure_tile(&self, tx: i32, tz: i32) -> std::io::Result<()> {
        if self.cache.read().await.contains_key(&tile_key(tx, tz)) {
            return Ok(());
        }
        let decoded = self
            .tiles
            .read_water_field(tx, tz)
            .await?
            .and_then(|raw| Self::decode_surfaces(&raw));
        let mut cache = self.cache.write().await;
        cache.entry(tile_key(tx, tz)).or_insert(decoded);
        Ok(())
    }

    /// Water surface height at a world position, bilinearly interpolated.
    /// `SEA_LEVEL` where no water field is baked. Loads tiles on demand.
    pub async fn sample_surface(&self, world_x: f32, world_z: f32) -> std::io::Result<f32> {
        let tx = world_to_tile(world_x);
        let tz = world_to_tile(world_z);

        self.ensure_tile(tx, tz).await?;

        let tile_min_x = tx as f32 * TILE_SIZE - TILE_SIZE / 2.0;
        let tile_min_z = tz as f32 * TILE_SIZE - TILE_SIZE / 2.0;
        let local_x = world_x - tile_min_x;
        let local_z = world_z - tile_min_z;

        let cell_x = local_x.floor() as i32;
        let cell_z = local_z.floor() as i32;

        if cell_x + 1 >= VERTS_PER_SIDE as i32 {
            let _ = self.ensure_tile(tx + 1, tz).await;
        }
        if cell_z + 1 >= VERTS_PER_SIDE as i32 {
            let _ = self.ensure_tile(tx, tz + 1).await;
        }

        let frac_x = local_x - local_x.floor();
        let frac_z = local_z - local_z.floor();

        let cache = self.cache.read().await;
        let s00 = get_surface_at_cell(&cache, tx, tz, cell_x, cell_z);
        let s10 = get_surface_at_cell(&cache, tx, tz, cell_x + 1, cell_z);
        let s01 = get_surface_at_cell(&cache, tx, tz, cell_x, cell_z + 1);
        let s11 = get_surface_at_cell(&cache, tx, tz, cell_x + 1, cell_z + 1);

        let s0 = s00 + (s10 - s00) * frac_x;
        let s1 = s01 + (s11 - s01) * frac_x;
        Ok(s0 + (s1 - s0) * frac_z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a WFD1 buffer whose every pixel carries `surface_m`.
    fn wfd1_uniform(surface_m: f32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(WFD_MAGIC);
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(VERTS_PER_SIDE as u16).to_le_bytes());
        out.extend_from_slice(&(VERTS_PER_SIDE as u16).to_le_bytes());
        out.extend_from_slice(&[0u8; 6]);
        let enc = ((surface_m + 500.0) / 0.05).round() as u16;
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
