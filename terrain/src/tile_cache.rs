//! Generation-swept tile cache shared by the height and water samplers.
//!
//! Reads stamp each entry with the cache's current generation (a relaxed
//! atomic store under the read lock, so concurrent sampling stays on the
//! read path). `sweep_stale` evicts entries not touched since the previous
//! sweep and advances the generation — callers running it on a period get
//! an effective idle TTL of one to two periods, with no clock to mock in
//! tests. A capacity fuse on insert bounds memory even between sweeps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{RwLock, RwLockReadGuard};

/// Capacity fuse per sampler, in tiles. Sized well above the working set a
/// full server implies (players cluster; a 5k-CCU worst case touches a few
/// thousand tiles between sweeps) so it only trips on pathological access —
/// tripping it is a sizing signal, not normal operation, hence the warn.
pub(crate) const TILE_CACHE_CAPACITY: usize = 16_384;

struct Entry<V> {
    value: V,
    generation: AtomicU64,
}

pub(crate) struct TileCache<V> {
    map: RwLock<HashMap<(i32, i32), Entry<V>>>,
    generation: AtomicU64,
    capacity: usize,
}

pub(crate) struct TileCacheReadGuard<'a, V> {
    map: RwLockReadGuard<'a, HashMap<(i32, i32), Entry<V>>>,
    generation: &'a AtomicU64,
}

impl<V> TileCacheReadGuard<'_, V> {
    /// Look up a tile, marking it live for the current sweep period.
    pub fn get(&self, key: &(i32, i32)) -> Option<&V> {
        let entry = self.map.get(key)?;
        entry
            .generation
            .store(self.generation.load(Ordering::Relaxed), Ordering::Relaxed);
        Some(&entry.value)
    }
}

impl<V> TileCache<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
            generation: AtomicU64::new(0),
            capacity,
        }
    }

    /// Snapshot for sampling; lookups through the guard touch entries.
    pub async fn read(&self) -> TileCacheReadGuard<'_, V> {
        TileCacheReadGuard {
            map: self.map.read().await,
            generation: &self.generation,
        }
    }

    /// True when the tile is cached; counts as a touch.
    pub async fn contains(&self, key: &(i32, i32)) -> bool {
        self.read().await.get(key).is_some()
    }

    /// Insert unless a concurrent loader won the race (first value sticks,
    /// mirroring the samplers' double-checked load). Trips the capacity fuse
    /// afterwards, evicting stalest-generation entries first.
    pub async fn insert_if_absent(&self, key: (i32, i32), value: V) {
        let mut map = self.map.write().await;
        let generation = self.generation.load(Ordering::Relaxed);
        map.entry(key).or_insert(Entry {
            value,
            generation: AtomicU64::new(generation),
        });

        if map.len() <= self.capacity {
            return;
        }
        let excess = map.len() - self.capacity;
        let mut evictable: Vec<((i32, i32), u64)> = map
            .iter()
            .map(|(k, e)| (*k, e.generation.load(Ordering::Relaxed)))
            .collect();
        evictable.sort_by_key(|&(_, gen)| gen);
        for (k, _) in evictable.into_iter().take(excess) {
            map.remove(&k);
        }
        tracing::warn!(
            "tile cache over capacity ({} tiles): evicted {excess} before their sweep — \
             consider raising TILE_CACHE_CAPACITY or sweeping more often",
            self.capacity
        );
    }

    pub async fn remove(&self, key: &(i32, i32)) {
        self.map.write().await.remove(key);
    }

    /// Evict every tile not touched since the previous sweep and start a new
    /// period. Returns the number of evicted tiles.
    pub async fn sweep_stale(&self) -> usize {
        let mut map = self.map.write().await;
        let current = self.generation.load(Ordering::Relaxed);
        let before = map.len();
        map.retain(|_, e| e.generation.load(Ordering::Relaxed) >= current);
        self.generation.store(current + 1, Ordering::Relaxed);
        before - map.len()
    }

    pub async fn len(&self) -> usize {
        self.map.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sweep_keeps_touched_tiles_and_evicts_idle_ones() {
        let cache = TileCache::new(TILE_CACHE_CAPACITY);
        cache.insert_if_absent((0, 0), 1u32).await;
        cache.insert_if_absent((1, 0), 2u32).await;

        // Fresh entries survive their first sweep.
        assert_eq!(cache.sweep_stale().await, 0);

        // Only (0,0) is touched this period.
        assert_eq!(cache.read().await.get(&(0, 0)), Some(&1));

        assert_eq!(cache.sweep_stale().await, 1);
        assert_eq!(cache.len().await, 1);
        assert!(cache.contains(&(0, 0)).await);
        assert!(!cache.contains(&(1, 0)).await);
    }

    #[tokio::test]
    async fn idle_tile_survives_one_full_period_after_its_last_touch() {
        let cache = TileCache::new(TILE_CACHE_CAPACITY);
        cache.insert_if_absent((0, 0), 1u32).await;

        // Untouched across one sweep boundary: still resident...
        assert_eq!(cache.sweep_stale().await, 0);
        // ...but gone after a second sweep with no touch in between.
        assert_eq!(cache.sweep_stale().await, 1);
        assert_eq!(cache.len().await, 0);
    }

    #[tokio::test]
    async fn capacity_fuse_evicts_stalest_generation_first() {
        let cache = TileCache::new(2);
        cache.insert_if_absent((0, 0), 1u32).await;
        cache.sweep_stale().await;
        // (0,0) now carries the previous generation; the newcomers are fresh.
        cache.insert_if_absent((1, 0), 2u32).await;
        cache.insert_if_absent((2, 0), 3u32).await;

        assert_eq!(cache.len().await, 2);
        assert!(!cache.contains(&(0, 0)).await);
        assert!(cache.contains(&(1, 0)).await);
        assert!(cache.contains(&(2, 0)).await);
    }

    #[tokio::test]
    async fn first_insert_wins_the_load_race() {
        let cache = TileCache::new(TILE_CACHE_CAPACITY);
        cache.insert_if_absent((0, 0), 1u32).await;
        cache.insert_if_absent((0, 0), 2u32).await;
        assert_eq!(cache.read().await.get(&(0, 0)), Some(&1));
    }
}
