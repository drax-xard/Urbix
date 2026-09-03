//! # api.rs
//!
//! Public, high-level API surface of the Urbix engine.
//!
//! This module historically planned to be the ergonomic facade for foreign
//! consumers. In practice the engine's public surface is now `engine::WorldEngine`
//! (Rust) and `ffi` (C, via `include/urbix.h`). This module is retained for
//! structural parity with the module map (`Urbix_Project.md §2.1`) and may host
//! future ergonomic helpers that are not `WorldEngine`-specific.
//!
//! For actual generation, see `engine::WorldEngine::generate_chunk`,
//! `engine::WorldEngine::get_zone_affinity`, and the `ffi` wrappers.

/// Re-export of the primary engine handle for `api`-level discoverability.
pub use crate::engine::WorldEngine;
