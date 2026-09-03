//! # engine.rs
//!
//! Stateful `WorldEngine` facade for the Urbix city engine.
//!
//! This module owns the engine's long-lived state — the world configuration,
//! the immutable Voronoi region map, and the LRU chunk cache — and presents a
//! single, ergonomic handle that ties the lower-level modules together.
//!
//! ## Responsibilities
//!
//! - Construct with a seed, generating the Voronoi district map once for the
//!   run.
//! - Generate chunks on demand via `chunk.rs`, caching them in `cache.rs`.
//! - Answer continuous zone queries via `region.rs`.
//! - Adjust live settings such as draw distance and chunk size.
//! - Evict far chunks so memory stays bounded during infinite exploration.
//!
//! ## Threading model
//!
//! Chunk generation is pure and per-chunk independent, so a future
//! multithreaded/worker-pool path (Milestone 8.8) is safe to add. The engine
//! handle itself is intended for single-user use but could be shared behind a
//! lock if needed.

use crate::cache::ChunkCache;
use crate::chunk::generate_chunk;
use crate::config::WorldConfig;
use crate::data::{ChunkBuffer, ChunkId};
use crate::region::VoronoiDiagram;
use crate::zones::ZONE_COUNT;

/// Stateful engine tying together config, Voronoi region, and chunk cache.
///
/// Construct with a seed; the Voronoi district map is generated once at
/// construction and kept for the entire run. Chunks are generated on demand
/// and cached with distance-based eviction so memory stays bounded.
///
/// ## Example
///
/// ```
/// use urbix::engine::WorldEngine;
///
/// let mut engine = WorldEngine::new(445566);
/// let chunk = engine.generate_chunk(0, 0);
/// assert_eq!(chunk.header().cx, 0);
/// assert_eq!(engine.generated_count(), 1);
/// // Second call for the same chunk hits the cache.
/// let same = engine.generate_chunk(0, 0);
/// assert_eq!(engine.generated_count(), 1); // no recomputation
/// assert_eq!(chunk.as_bytes(), same.as_bytes());
/// ```
pub struct WorldEngine {
    config: WorldConfig,
    voronoi: VoronoiDiagram,
    cache: ChunkCache,
    /// Number of chunks that actually went through the generation pipeline
    /// (cache misses). Cache hits are not counted here so this metric
    /// directly reflects generation work.
    generated_count: u64,
}

impl WorldEngine {
    /// Construct an engine with the given seed and default configuration.
    ///
    /// The Voronoi district map is generated once from `(seed,
    /// voronoi_site_count)` and held for the run. The initial draw distance
    /// comes from `WorldConfig::default().draw_distance`.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::engine::WorldEngine;
    ///
    /// let engine = WorldEngine::new(12345);
    /// assert_eq!(engine.config().seed, 12345);
    /// ```
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self::with_config(WorldConfig {
            seed,
            ..Default::default()
        })
    }

    /// Construct an engine from a fully-specified [`WorldConfig`].
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::engine::WorldEngine;
    /// use urbix::config::WorldConfig;
    ///
    /// let cfg = WorldConfig { seed: 7, draw_distance: 16, ..Default::default() };
    /// let engine = WorldEngine::with_config(cfg);
    /// assert_eq!(engine.config().draw_distance, 16);
    /// ```
    #[must_use]
    pub fn with_config(config: WorldConfig) -> Self {
        let voronoi = VoronoiDiagram::generate(config.seed, config.voronoi_site_count);
        let center = ChunkId::new(0, 0);
        let cache = ChunkCache::new(center, config.draw_distance);
        Self {
            config,
            voronoi,
            cache,
            generated_count: 0,
        }
    }

    /// Generate (or retrieve from cache) the chunk at `(cx, cy)`.
    ///
    /// On a cache hit the stored copy is touched (LRU update) and returned
    /// without recomputation. On a miss the chunk is generated, inserted,
    /// and distant chunks are evicted to keep memory bounded.
    ///
    /// Returns an owned [`ChunkBuffer`] — consumers can read and discard it,
    /// or hold it for rendering/inspection.
    pub fn generate_chunk(&mut self, cx: i32, cy: i32) -> ChunkBuffer {
        let id = ChunkId::new(cx, cy);

        // Cache hit path.
        if let Some(cached) = self.cache.get(id) {
            return cached.clone();
        }

        // Cache miss: generate and insert.
        self.generated_count += 1;
        let buf = generate_chunk(cx, cy, &self.config, &self.voronoi);
        self.cache.insert(id, buf.clone());
        self.evict_distant_chunks();
        buf
    }

    /// Query the continuous zone-affinity vector at world cell coordinates.
    ///
    /// Delegates directly to the immutable Voronoi diagram. The result is a
    /// length-[`ZONE_COUNT`] weight vector (one entry per [`ZoneType`])
    /// summing to 1.0.
    pub fn get_zone_affinity(&self, world_x: f64, world_z: f64) -> [f32; ZONE_COUNT] {
        self.voronoi.query(world_x, world_z)
    }

    /// Update the draw distance (in chunk Chebyshev units).
    ///
    /// Takes effect on the next call to [`evict_distant_chunks`](Self::evict_distant_chunks)
    /// (which runs automatically after each generation).
    pub fn set_draw_distance(&mut self, dd: u32) {
        self.cache.set_draw_distance(dd);
    }

    /// Change the cells-per-side chunk size for subsequent generation.
    ///
    /// Cached chunks were generated at the previous size — their cell counts
    /// and on-wire layout no longer match what new chunks will produce — so
    /// the cache is cleared, and the new size takes effect on the next
    /// [`generate_chunk`](Self::generate_chunk) call.
    ///
    /// ## Panics
    ///
    /// Panics if `size` is zero (a chunk must contain at least one cell).
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::engine::WorldEngine;
    ///
    /// let mut engine = WorldEngine::new(445566);
    /// engine.set_chunk_size(16);
    /// assert_eq!(engine.config().chunk_size, 16);
    /// let chunk = engine.generate_chunk(0, 0);
    /// assert_eq!(chunk.header().chunk_size, 16);
    /// assert_eq!(chunk.cell_count(), 16 * 16);
    /// ```
    pub fn set_chunk_size(&mut self, size: u16) {
        assert!(size > 0, "chunk size must be non-zero");
        self.config.chunk_size = size;
        // All cached buffers were built at the old size and are no longer
        // consistent with the new size; drop them to avoid mixing layouts.
        self.cache.clear();
    }

    /// Update the engine's center chunk coordinate.
    ///
    /// Takes effect on the next eviction cycle.
    pub fn set_center(&mut self, cx: i32, cy: i32) {
        self.cache.set_center(ChunkId::new(cx, cy));
    }

    /// Manually trigger eviction of all chunks beyond the current draw
    /// distance. Normally runs automatically after each generation.
    pub fn evict_distant_chunks(&mut self) {
        self.cache.evict_distant_chunks();
    }

    /// Read-only reference to the world configuration.
    #[must_use]
    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    /// Number of chunks that were actually generated (cache misses).
    #[must_use]
    pub fn generated_count(&self) -> u64 {
        self.generated_count
    }

    /// Number of chunks currently held in the cache.
    #[must_use]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Bytes of heap memory held by cached chunk buffers.
    #[must_use]
    pub fn cache_memory_bytes(&self) -> usize {
        self.cache.memory_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_engine_has_default_config() {
        let engine = WorldEngine::new(42);
        assert_eq!(engine.config().seed, 42);
        assert_eq!(engine.config().chunk_size, 32);
        assert_eq!(engine.config().draw_distance, 8);
        assert_eq!(engine.generated_count(), 0);
        assert!(engine.cache_len() == 0);
    }

    #[test]
    fn generate_chunk_populates_cache_and_is_deterministic() {
        let mut engine = WorldEngine::new(445566);
        let a = engine.generate_chunk(0, 0);
        let b = engine.generate_chunk(0, 0);
        assert_eq!(a.as_bytes(), b.as_bytes());
        assert_eq!(engine.generated_count(), 1); // second call is a hit
        assert_eq!(engine.cache_len(), 1);
    }

    #[test]
    fn cached_chunk_is_not_recomputed() {
        let mut engine = WorldEngine::new(1234);
        let _ = engine.generate_chunk(3, 7);
        assert_eq!(engine.generated_count(), 1);
        // Generate the same chunk again.
        let _ = engine.generate_chunk(3, 7);
        assert_eq!(engine.generated_count(), 1); // no recomputation
                                                 // Generate a different chunk.
        let _ = engine.generate_chunk(4, 7);
        assert_eq!(engine.generated_count(), 2);
    }

    #[test]
    fn evict_distant_chunks_removes_far_entries() {
        let mut engine = WorldEngine::new(99);
        // Default draw_distance is 8. Generate chunks around center.
        let _ = engine.generate_chunk(0, 0);
        let _ = engine.generate_chunk(1, 0);
        let _ = engine.generate_chunk(0, 1);
        assert_eq!(engine.cache_len(), 3);

        // Move center far away and generate a new chunk.
        engine.set_center(100, 100);
        let _ = engine.generate_chunk(100, 100);
        // Old chunks are now at distance ~100, far beyond dd=8; auto-evicted.
        assert_eq!(engine.cache_len(), 1);
    }

    #[test]
    fn draw_distance_controls_eviction() {
        let mut engine = WorldEngine::new(99);
        let _ = engine.generate_chunk(5, 5); // distance 5 from center (0,0)

        // dd=4: chunk (5,5) is beyond; evict it.
        engine.set_draw_distance(4);
        engine.evict_distant_chunks();
        assert_eq!(engine.cache_len(), 0);

        // Expand dd: now it survives.
        engine.set_draw_distance(6);
        let _ = engine.generate_chunk(5, 5);
        assert_eq!(engine.cache_len(), 1);
        engine.evict_distant_chunks();
        assert_eq!(engine.cache_len(), 1); // still within range
    }

    #[test]
    fn bounded_memory_over_simulated_walk() {
        // Simulate a 1000-step linear walk: move center each step, generate
        // a chunk at the new center, and verify the cache never grows beyond
        // `(2*dd+1)^2 + 1` (the draw distance square plus one for the newly
        // generated chunk).
        let mut engine = WorldEngine::new(42);
        let dd = engine.config().draw_distance;
        let max_capacity = (2 * dd + 1) * (2 * dd + 1) + 1;

        for step in 0u32..1000 {
            // Move east along the x axis.
            let cx = step as i32;
            engine.set_center(cx, 0);
            let _ = engine.generate_chunk(cx, 0);
            assert!(
                engine.cache_len() as u32 <= max_capacity,
                "step {step}: cache_len={} exceeds {max_capacity}",
                engine.cache_len()
            );
        }
        // Final cache size should be at most the draw-distance window.
        assert!(
            engine.cache_len() as u32 <= max_capacity,
            "final cache_len={} exceeds {max_capacity}",
            engine.cache_len()
        );
    }

    #[test]
    fn get_zone_affinity_returns_valid_weights() {
        let engine = WorldEngine::new(445566);
        let affinity = engine.get_zone_affinity(100.0, 200.0);
        assert_eq!(affinity.len(), ZONE_COUNT);
        let sum: f32 = affinity.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "affinity sum={sum}");
        assert!(affinity.iter().all(|w| *w >= 0.0));
    }

    #[test]
    fn engine_with_custom_config() {
        let cfg = WorldConfig {
            seed: 7,
            draw_distance: 3,
            chunk_size: 16,
            ..Default::default()
        };
        let mut engine = WorldEngine::with_config(cfg);
        assert_eq!(engine.config().draw_distance, 3);
        assert_eq!(engine.config().chunk_size, 16);

        // With small dd, distant chunks are quickly evicted.
        let _ = engine.generate_chunk(0, 0);
        engine.set_center(10, 10);
        let _ = engine.generate_chunk(10, 10);
        assert_eq!(engine.cache_len(), 1);
    }

    #[test]
    fn set_chunk_size_applies_to_later_generation() {
        let mut engine = WorldEngine::new(7);
        // One chunk at the default size.
        let _ = engine.generate_chunk(0, 0);
        assert_eq!(engine.config().chunk_size, 32);

        engine.set_chunk_size(64);
        assert_eq!(engine.config().chunk_size, 64);
        let chunk = engine.generate_chunk(0, 0);
        assert_eq!(chunk.header().chunk_size, 64);
        assert_eq!(chunk.cell_count(), 64 * 64);
    }

    #[test]
    fn set_chunk_size_clears_cached_chunks() {
        let mut engine = WorldEngine::new(7);
        let _ = engine.generate_chunk(1, 2);
        assert_eq!(engine.cache_len(), 1);

        // The new size must not silently serve stale, old-size buffers.
        engine.set_chunk_size(16);
        assert_eq!(engine.cache_len(), 0);

        // After regeneration the cache reflects the new size.
        let chunk = engine.generate_chunk(1, 2);
        assert_eq!(chunk.header().chunk_size, 16);
        assert_eq!(engine.cache_len(), 1);
    }

    #[test]
    #[should_panic(expected = "chunk size must be non-zero")]
    fn set_chunk_size_rejects_zero() {
        let mut engine = WorldEngine::new(7);
        engine.set_chunk_size(0);
    }
}
