use crate::coords::world_to_tile;
use crate::defaults::{self, VERTS_PER_SIDE};
use crate::io::TerrainIO;
use crate::tile_cache::{TileCache, TileCacheReadGuard, TILE_CACHE_CAPACITY};

/// Tile size in world units (must match client TERRAIN_TILE_SIZE).
const TILE_SIZE: f32 = defaults::TILE_DIM as f32;

/// Decode a uint16 heightmap value to meters.
/// Encoding: `round((meters + 500.0) / 0.05)` → range -500m to +3276m.
/// Also the water field's surfaceY codec.
pub(crate) fn decode_height(value: u16) -> f32 {
    value as f32 * 0.05 - 500.0
}

/// Resolve a possibly-out-of-range tile-local vertex to its owning tile and
/// row-major index. Each tile stores the edge vertex it shares with its
/// neighbour, so ±1 steps cross at most one tile.
pub(crate) fn resolve_cell(tx: i32, tz: i32, cell_x: i32, cell_z: i32) -> ((i32, i32), usize) {
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
    ((tx, tz), cz as usize * VERTS_PER_SIDE + cx as usize)
}

/// Bilinear interpolation of per-vertex values supplied by `get(tx, tz, cx, cz)`.
pub(crate) fn bilinear(world_x: f32, world_z: f32, get: impl Fn(i32, i32, i32, i32) -> f32) -> f32 {
    let tx = world_to_tile(world_x);
    let tz = world_to_tile(world_z);
    let local_x = world_x - (tx as f32 * TILE_SIZE - TILE_SIZE / 2.0);
    let local_z = world_z - (tz as f32 * TILE_SIZE - TILE_SIZE / 2.0);
    let cell_x = local_x.floor() as i32;
    let cell_z = local_z.floor() as i32;
    let frac_x = local_x - local_x.floor();
    let frac_z = local_z - local_z.floor();

    let v00 = get(tx, tz, cell_x, cell_z);
    let v10 = get(tx, tz, cell_x + 1, cell_z);
    let v01 = get(tx, tz, cell_x, cell_z + 1);
    let v11 = get(tx, tz, cell_x + 1, cell_z + 1);

    let v0 = v00 + (v10 - v00) * frac_x;
    let v1 = v01 + (v11 - v01) * frac_x;
    v0 + (v1 - v0) * frac_z
}

/// Height at a tile-local vertex from a cache snapshot; a miss reads as 0.0.
fn get_height_at_cell(
    cache: &TileCacheReadGuard<'_, Vec<u16>>,
    tx: i32,
    tz: i32,
    cell_x: i32,
    cell_z: i32,
) -> f32 {
    let (key, idx) = resolve_cell(tx, tz, cell_x, cell_z);
    cache
        .get(&key)
        .and_then(|heights| heights.get(idx))
        .map_or(0.0, |v| decode_height(*v))
}

fn sample_cached(cache: &TileCacheReadGuard<'_, Vec<u16>>, world_x: f32, world_z: f32) -> f32 {
    bilinear(world_x, world_z, |tx, tz, cx, cz| {
        get_height_at_cell(cache, tx, tz, cx, cz)
    })
}

/// Where raw heightmap tiles come from. The local data directory when the
/// caller sits on the game server; something else (the server's public tile
/// API) for clients running elsewhere, which cannot carry the 3 GB tree.
#[async_trait::async_trait]
pub trait HeightTiles: Send + Sync {
    /// Raw little-endian u16 heightmap for one tile, `HEIGHTMAP_SIZE` bytes.
    /// Missing tiles yield `defaults::default_heightmap()` rather than an
    /// error — the world is larger than the baked area.
    async fn read_heightmap(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>>;
}

#[async_trait::async_trait]
impl HeightTiles for TerrainIO {
    async fn read_heightmap(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
        TerrainIO::read_heightmap(self, tx, tz).await
    }
}

/// Provides terrain height sampling with an in-memory tile cache.
/// Loads heightmap tiles on demand from a `HeightTiles` source and caches them.
///
/// Uses interior mutability (`tokio::sync::RwLock`) so callers only need `&self`,
/// avoiding external mutex contention when multiple NPC connections share one sampler.
pub struct HeightSampler {
    cache: TileCache<Vec<u16>>,
    tiles: Box<dyn HeightTiles>,
}

impl HeightSampler {
    pub fn new(tiles: impl HeightTiles + 'static) -> Self {
        Self {
            cache: TileCache::new(TILE_CACHE_CAPACITY),
            tiles: Box::new(tiles),
        }
    }

    /// Ensure a tile's heightmap is loaded into the cache.
    /// No lock held during I/O; re-checks before decoding, first insert wins.
    async fn ensure_tile(&self, tx: i32, tz: i32) -> std::io::Result<()> {
        if self.cache.contains(&(tx, tz)).await {
            return Ok(());
        }
        let raw = self.tiles.read_heightmap(tx, tz).await?;
        if self.cache.contains(&(tx, tz)).await {
            return Ok(());
        }
        let heights: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        self.cache.insert_if_absent((tx, tz), heights).await;
        Ok(())
    }

    /// Sample terrain height at an arbitrary world position using bilinear
    /// interpolation, loading the covering tile on demand. One tile covers all
    /// four corners: `VERTS_PER_SIDE` is `TILE_DIM + 1`, so each tile stores the
    /// edge vertex it shares with its neighbour.
    pub async fn sample_height(&self, world_x: f32, world_z: f32) -> std::io::Result<f32> {
        self.ensure_tile(world_to_tile(world_x), world_to_tile(world_z))
            .await?;
        let cache = self.cache.read().await;
        Ok(sample_cached(&cache, world_x, world_z))
    }

    /// Number of tiles currently cached.
    pub async fn cached_tile_count(&self) -> usize {
        self.cache.len().await
    }

    /// Evict tiles not sampled since the previous sweep.
    pub async fn sweep_stale_tiles(&self) -> usize {
        self.cache.sweep_stale().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Tiles whose heights vary with both tile and cell index, so a grid that
    /// mis-attributes a sample to the wrong tile or cell shows up as a mismatch.
    struct CountingTiles(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl HeightTiles for CountingTiles {
        async fn read_heightmap(&self, tx: i32, tz: i32) -> std::io::Result<Vec<u8>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            let mut out = Vec::with_capacity(defaults::HEIGHTMAP_SIZE);
            for cz in 0..VERTS_PER_SIDE as i32 {
                for cx in 0..VERTS_PER_SIDE as i32 {
                    let v = 10000 + tx * 37 + tz * 11 + cx * 3 + cz;
                    out.extend_from_slice(&(v as u16).to_le_bytes());
                }
            }
            Ok(out)
        }
    }

    fn counting_sampler() -> (HeightSampler, Arc<AtomicUsize>) {
        let reads = Arc::new(AtomicUsize::new(0));
        (HeightSampler::new(CountingTiles(Arc::clone(&reads))), reads)
    }

    #[tokio::test]
    async fn sample_height_covers_a_cell_from_one_tile() {
        // Each tile stores VERTS_PER_SIDE = TILE_DIM + 1 vertices, so all four
        // bilinear corners live in the covering tile — no neighbour load.
        let (s, reads) = counting_sampler();
        for w in [-32.0, -31.9, 0.0, 31.9, 32.0, 95.9, -1000.5, 4740.5] {
            assert!(s.sample_height(w, w).await.is_ok());
        }
        // One read per distinct tile touched, never a neighbour on top.
        let tiles: std::collections::HashSet<i32> =
            [-32.0f32, -31.9, 0.0, 31.9, 32.0, 95.9, -1000.5, 4740.5]
                .iter()
                .map(|w| world_to_tile(*w))
                .collect();
        assert_eq!(reads.load(Ordering::Relaxed), tiles.len());
    }

    #[tokio::test]
    async fn swept_tile_is_reloaded_from_the_source_on_next_sample() {
        let (s, reads) = counting_sampler();
        assert!(s.sample_height(0.0, 0.0).await.is_ok());
        assert_eq!(reads.load(Ordering::Relaxed), 1);

        // Idle across two sweeps: evicted; the next sample hits the source.
        s.sweep_stale_tiles().await;
        assert_eq!(s.sweep_stale_tiles().await, 1);
        assert_eq!(s.cached_tile_count().await, 0);

        assert!(s.sample_height(0.0, 0.0).await.is_ok());
        assert_eq!(reads.load(Ordering::Relaxed), 2);

        // Sampled this period: the next sweep keeps it, and no re-read occurs.
        s.sweep_stale_tiles().await;
        assert!(s.sample_height(0.0, 0.0).await.is_ok());
        assert_eq!(reads.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn decode_sea_level() {
        assert!((decode_height(10000) - 0.0).abs() < 0.001);
    }

    #[test]
    fn decode_negative() {
        // 6000 → 6000 * 0.05 - 500 = -200.0
        assert!((decode_height(6000) - (-200.0)).abs() < 0.001);
    }

    #[test]
    fn world_to_tile_center() {
        // Position (0, 0) should be tile (0, 0)
        assert_eq!(world_to_tile(0.0), 0);
    }

    #[test]
    fn world_to_tile_boundary() {
        // Tile 0 spans [-32, 32), tile 1 spans [32, 96)
        assert_eq!(world_to_tile(31.9), 0);
        assert_eq!(world_to_tile(32.0), 1);
        assert_eq!(world_to_tile(-32.0), 0);
        assert_eq!(world_to_tile(-32.1), -1);
    }
}
