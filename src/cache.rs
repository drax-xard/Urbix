//! # cache.rs
//!
//! Distance-based LRU cache for generated city chunks.
//!
//! The city is infinite and generated on demand, so without eviction the
//! engine's memory would grow without bound as a player explores. This module
//! holds materialized chunks in an LRU cache keyed by `ChunkId` and evicts
//! any chunk whose Chebyshev distance from the current center exceeds the
//! configured draw distance.
//!
//! ## Key invariant
//!
//! Because every chunk is deterministic from `hash(cx, cy, seed, ...)`, an
//! evicted chunk can always be regenerated identically. This makes eviction
//! completely safe: dropping a chunk costs only a future re-generation, never
//! a correctness change.
//!
//! ## Design
//!
//! - Keys: `ChunkId` (from `data.rs`).
//! - Values: materialized chunk buffers.
//! - Eviction policy: distance-based (drop far chunks) combined with LRU
//!   ordering (drop oldest first among candidates).
//! - Implementation: `HashMap<ChunkId, Entry>` with a monotonically increasing
//!   recency counter — no linked list needed. O(1) insert/get, O(n) eviction
//!   scan (acceptable since the active cache is bounded by `(2*dd+1)^2`).

use std::collections::HashMap;

use crate::data::{ChunkBuffer, ChunkId};

/// An entry in the LRU cache with a recency stamp.
#[derive(Clone, Debug)]
struct Entry {
    value: ChunkBuffer,
    /// Monotonically increasing recency stamp; higher = more recently used.
    last_used: u64,
}

/// LRU cache for generated chunk buffers, keyed by `ChunkId`.
///
/// The cache tracks a "center" chunk and a `draw_distance` (in chunk units).
/// On eviction, every chunk at Chebyshev distance `> draw_distance` from the
/// center is dropped. Among those, the least-recently-used are evicted first
/// (though in practice all candidates are evicted since distance is a hard
/// cutoff). An optional `capacity` limits total entries so memory stays bounded
/// even during long runs.
///
/// ## Example
///
/// ```
/// use urbix::cache::ChunkCache;
/// use urbix::data::{ChunkBuffer, ChunkId};
///
/// let mut cache = ChunkCache::new(ChunkId::new(0, 0), 8);
/// assert!(cache.is_empty());
/// assert_eq!(cache.len(), 0);
/// ```
#[derive(Debug)]
pub struct ChunkCache {
    map: HashMap<ChunkId, Entry>,
    center: ChunkId,
    draw_distance: u32,
    /// Maximum number of entries allowed. `usize::MAX` means unlimited.
    capacity: usize,
    /// Monotonically increasing recency stamp.
    tick: u64,
}

impl ChunkCache {
    /// Create a new empty cache centered on `center` with the given draw
    /// distance (in chunk Chebyshev units). No capacity limit by default.
    #[must_use]
    pub fn new(center: ChunkId, draw_distance: u32) -> Self {
        Self {
            map: HashMap::new(),
            center,
            draw_distance,
            capacity: usize::MAX,
            tick: 0,
        }
    }

    /// Set an optional hard capacity cap on the number of cached chunks.
    /// When the cache exceeds this size *after* distance eviction, the
    /// least-recently-used entries are dropped until the cap is met. Pass
    /// `None` for unlimited.
    pub fn set_capacity(&mut self, cap: Option<usize>) {
        self.capacity = cap.unwrap_or(usize::MAX);
    }

    /// Update the draw distance (in chunk Chebyshev units). Does **not**
    /// immediately evict — call [`evict_distant_chunks`](Self::evict_distant_chunks)
    /// or rely on the engine's auto-eviction after insertion.
    pub fn set_draw_distance(&mut self, dd: u32) {
        self.draw_distance = dd;
    }

    /// Return the current draw distance.
    #[must_use]
    pub fn draw_distance(&self) -> u32 {
        self.draw_distance
    }

    /// Update the center chunk coordinate. Does **not** immediately evict.
    pub fn set_center(&mut self, center: ChunkId) {
        self.center = center;
    }

    /// Return the current center chunk coordinate.
    #[must_use]
    pub fn center(&self) -> ChunkId {
        self.center
    }

    /// Look up a cached chunk by id and touch its recency stamp (LRU update).
    ///
    /// Returns a reference to the buffer if present, or `None` on miss.
    pub fn get(&mut self, key: ChunkId) -> Option<&ChunkBuffer> {
        if let Some(entry) = self.map.get_mut(&key) {
            self.tick += 1;
            entry.last_used = self.tick;
            Some(&entry.value)
        } else {
            None
        }
    }

    /// Insert a chunk buffer, touching its recency stamp.
    ///
    /// If the key already existed the old buffer is replaced and its recency
    /// stamp is reset. Does **not** evict — the caller (typically the engine)
    /// should call [`evict_distant_chunks`](Self::evict_distant_chunks) after
    /// insertion to keep memory bounded.
    pub fn insert(&mut self, key: ChunkId, value: ChunkBuffer) {
        self.tick += 1;
        self.map.insert(
            key,
            Entry {
                value,
                last_used: self.tick,
            },
        );
    }

    /// Evict every chunk whose Chebyshev distance from the current center
    /// exceeds `draw_distance`. After distance eviction, if a capacity cap is
    /// set, the least-recently-used excess entries are dropped as well.
    pub fn evict_distant_chunks(&mut self) {
        let dd = self.draw_distance;
        let c = self.center;

        // Phase 1: distance-based eviction.
        self.map.retain(|id, _| chebyshev(id, &c) <= dd);

        // Phase 2: LRU capacity cap (if set).
        if self.map.len() > self.capacity {
            // Collect and sort by recency ascending (oldest first), then drop
            // the excess.
            let mut entries: Vec<_> = self.map.iter().map(|(k, e)| (*k, e.last_used)).collect();
            entries.sort_by_key(|(_, t)| *t);
            let excess = self.map.len() - self.capacity;
            for (k, _) in entries.iter().take(excess) {
                self.map.remove(k);
            }
        }
    }

    /// Number of chunks currently in the cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Bytes of memory held by the cached chunks (not counting map overhead).
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.map.values().map(|e| e.value.as_bytes().len()).sum()
    }

    /// Iterate over cached chunk ids in arbitrary order (for diagnostics).
    pub fn keys(&self) -> impl Iterator<Item = ChunkId> + '_ {
        self.map.keys().copied()
    }
}

/// Chebyshev distance between two `ChunkId`s.
///
/// The component differences are computed in `i64` so that two chunk
/// coordinates at the extremes of the `i32` range (e.g. `i32::MIN` and
/// `i32::MAX`) produce their true ~4.29b distance instead of overflowing and
/// (in debug builds) panicking.
fn chebyshev(a: &ChunkId, b: &ChunkId) -> u32 {
    let dx = i64::from(a.cx).abs_diff(i64::from(b.cx));
    let dy = i64::from(a.cy).abs_diff(i64::from(b.cy));
    // Max possible |delta| across i32 is 2^32-1, which fits in u32.
    dx.max(dy) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;
    use crate::region::VoronoiDiagram;

    fn dummy_chunk(cx: i32, cy: i32) -> ChunkBuffer {
        let cfg = WorldConfig::default();
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
        crate::chunk::generate_chunk(cx, cy, &cfg, &voronoi)
    }

    #[test]
    fn insert_and_get() {
        let mut cache = ChunkCache::new(ChunkId::new(0, 0), 4);
        let id = ChunkId::new(2, 1);
        cache.insert(id, dummy_chunk(2, 1));
        assert_eq!(cache.len(), 1);
        assert!(cache.get(id).is_some());
        assert_eq!(cache.get(id).unwrap().header().cx, 2);
    }

    #[test]
    fn get_miss_returns_none() {
        let mut cache = ChunkCache::new(ChunkId::new(0, 0), 4);
        assert!(cache.get(ChunkId::new(0, 0)).is_none());
    }

    #[test]
    fn evict_distant_chunks_drops_far_entries() {
        let mut cache = ChunkCache::new(ChunkId::new(0, 0), 2);
        // Within draw distance.
        cache.insert(ChunkId::new(1, 1), dummy_chunk(1, 1));
        cache.insert(ChunkId::new(2, 0), dummy_chunk(2, 0));
        // Beyond draw distance.
        cache.insert(ChunkId::new(5, 5), dummy_chunk(5, 5));
        cache.insert(ChunkId::new(0, 4), dummy_chunk(0, 4));
        assert_eq!(cache.len(), 4);
        cache.evict_distant_chunks();
        assert_eq!(cache.len(), 2);
        assert!(cache.get(ChunkId::new(1, 1)).is_some());
        assert!(cache.get(ChunkId::new(5, 5)).is_none());
        assert!(cache.get(ChunkId::new(0, 4)).is_none());
    }

    #[test]
    fn evict_respects_negative_coordinates() {
        let mut cache = ChunkCache::new(ChunkId::new(-1, -1), 1);
        cache.insert(ChunkId::new(-2, -2), dummy_chunk(-2, -2)); // distance 1: keep
        cache.insert(ChunkId::new(0, -1), dummy_chunk(0, -1)); // distance 1: keep
        cache.insert(ChunkId::new(-5, -1), dummy_chunk(-5, -1)); // distance 4: drop
        cache.evict_distant_chunks();
        assert_eq!(cache.len(), 2);
        assert!(cache.get(ChunkId::new(-2, -2)).is_some());
        assert!(cache.get(ChunkId::new(-5, -1)).is_none());
    }

    #[test]
    fn set_center_and_draw_distance() {
        let mut cache = ChunkCache::new(ChunkId::new(0, 0), 1);
        cache.insert(ChunkId::new(2, 0), dummy_chunk(2, 0));
        cache.evict_distant_chunks();
        assert_eq!(cache.len(), 0); // distance 2 > dd 1

        // Expand draw distance and re-add — then it survives.
        cache.set_draw_distance(3);
        cache.insert(ChunkId::new(2, 0), dummy_chunk(2, 0));
        cache.evict_distant_chunks();
        assert_eq!(cache.len(), 1);

        // Move center — old chunk may now be far.
        cache.set_center(ChunkId::new(5, 5));
        cache.evict_distant_chunks();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn lru_recency_on_get() {
        let mut cache = ChunkCache::new(ChunkId::new(0, 0), 5);
        cache.set_capacity(Some(2));
        // Insert two entries.
        cache.insert(ChunkId::new(0, 0), dummy_chunk(0, 0));
        cache.insert(ChunkId::new(1, 0), dummy_chunk(1, 0));
        // Touch the first one (updates recency).
        let _ = cache.get(ChunkId::new(0, 0));
        // Insert a third — capacity forces eviction of the *oldest*.
        cache.insert(ChunkId::new(2, 0), dummy_chunk(2, 0));
        cache.evict_distant_chunks(); // distance is fine, but capacity triggers LRU drop
        assert_eq!(cache.len(), 2);
        // (0,0) was touched most recently, so (1,0) should be the victim.
        assert!(cache.get(ChunkId::new(1, 0)).is_none());
        assert!(cache.get(ChunkId::new(0, 0)).is_some());
    }

    #[test]
    fn capacity_lru_works_with_zero_entries() {
        let mut cache = ChunkCache::new(ChunkId::new(0, 0), 100);
        cache.set_capacity(Some(0));
        cache.insert(ChunkId::new(0, 0), dummy_chunk(0, 0));
        cache.evict_distant_chunks();
        assert!(cache.is_empty());
    }

    #[test]
    fn chebyshev_distance_cases() {
        // Negative coordinates: (-5,-5) to (0,0) = distance 5.
        let a = ChunkId::new(-5, -5);
        let b = ChunkId::new(0, 0);
        assert_eq!(chebyshev(&a, &b), 5);
        assert_eq!(chebyshev(&b, &a), 5);
        // Same point.
        assert_eq!(chebyshev(&ChunkId::new(7, -3), &ChunkId::new(7, -3)), 0);
        // Extremes of the i32 range: the true span (2^32-1) must not overflow.
        assert_eq!(
            chebyshev(&ChunkId::new(i32::MIN, 0), &ChunkId::new(i32::MAX, 0)),
            u32::MAX
        );
        // Mixed-sign extremes on both axes.
        assert_eq!(
            chebyshev(
                &ChunkId::new(i32::MIN, i32::MIN),
                &ChunkId::new(i32::MAX, i32::MAX)
            ),
            u32::MAX
        );
    }
}
