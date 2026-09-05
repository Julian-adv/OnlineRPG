//! Generation-swept tile cache shared by the height and water samplers.
//! Reads stamp entries with the current generation under the read lock;
//! `sweep_stale` evicts entries untouched since the previous sweep, giving
//! an idle TTL of one to two periods. A capacity fuse bounds inserts.

use std::collections::hash_map::Entry as MapEntry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{RwLock, RwLockReadGuard};

/// Well above any live working set; tripping it is a sizing signal.
pub(crate) const TILE_CACHE_CAPACITY: usize = 16_384;

/// Sweep cadence the idle TTL and `TILE_CACHE_CAPACITY` are sized against.
pub const TILE_CACHE_SWEEP_PERIOD: Duration = Duration::from_secs(300);

struct Entry<V> {
    value: V,
    generation: AtomicU64,
}

pub(crate) struct TileCache<V> {
    map: RwLock<HashMap<(i32, i32), Entry<V>>>,
    generation: AtomicU64,
    capacity: usize,
    fuse_warned: AtomicBool,
}

pub(crate) struct TileCacheReadGuard<'a, V> {
    map: RwLockReadGuard<'a, HashMap<(i32, i32), Entry<V>>>,
    generation: u64,
}

impl<V> TileCacheReadGuard<'_, V> {
    /// Look up a tile, marking it live for the current sweep period.
    pub fn get(&self, key: &(i32, i32)) -> Option<&V> {
        let entry = self.map.get(key)?;
        if entry.generation.load(Ordering::Relaxed) != self.generation {
            entry.generation.store(self.generation, Ordering::Relaxed);
        }
        Some(&entry.value)
    }
}

impl<V> TileCache<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
            generation: AtomicU64::new(0),
            capacity,
            fuse_warned: AtomicBool::new(false),
        }
    }

    /// Snapshot for sampling; lookups through the guard touch entries.
    pub async fn read(&self) -> TileCacheReadGuard<'_, V> {
        let map = self.map.read().await;
        TileCacheReadGuard {
            map,
            generation: self.generation.load(Ordering::Relaxed),
        }
    }

    /// True when the tile is cached; does not touch the entry.
    pub async fn contains(&self, key: &(i32, i32)) -> bool {
        self.map.read().await.contains_key(key)
    }

    /// Insert unless a concurrent loader won the race (first value sticks).
    /// Over capacity, evicts the stalest-generation entry.
    pub async fn insert_if_absent(&self, key: (i32, i32), value: V) {
        self.insert(key, value, false).await;
    }

    pub async fn replace(&self, key: (i32, i32), value: V) {
        self.insert(key, value, true).await;
    }

    async fn insert(&self, key: (i32, i32), value: V, replace: bool) {
        let mut map = self.map.write().await;
        let value = Entry {
            value,
            generation: AtomicU64::new(self.generation.load(Ordering::Relaxed)),
        };
        match map.entry(key) {
            MapEntry::Occupied(mut slot) => {
                if replace {
                    slot.insert(value);
                }
                return;
            }
            MapEntry::Vacant(slot) => slot.insert(value),
        };

        if map.len() <= self.capacity {
            return;
        }
        let victim = map
            .iter()
            .min_by_key(|(_, e)| e.generation.load(Ordering::Relaxed))
            .map(|(k, _)| *k);
        if let Some(k) = victim {
            map.remove(&k);
        }
        if !self.fuse_warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "tile cache over capacity ({} tiles): evicting stalest tiles",
                self.capacity
            );
        }
    }

    /// Evict every tile not touched since the previous sweep and start a new
    /// period. Returns the number of evicted tiles.
    pub async fn sweep_stale(&self) -> usize {
        let mut map = self.map.write().await;
        let current = self.generation.load(Ordering::Relaxed);
        let before = map.len();
        map.retain(|_, e| e.generation.load(Ordering::Relaxed) >= current);
        self.generation.store(current + 1, Ordering::Relaxed);
        self.fuse_warned.store(false, Ordering::Relaxed);
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
