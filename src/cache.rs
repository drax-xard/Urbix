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

// TODO(Milestone 4): implement the LRU cache with distance-based eviction.
