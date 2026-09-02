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

// TODO(Milestone 4): implement WorldEngine.
