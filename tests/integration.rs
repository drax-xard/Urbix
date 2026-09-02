//! Integration tests for the Urbix engine.
//!
//! This directory holds end-to-end tests that exercise the engine through its
//! public API rather than internal modules. Integration tests are compiled as
//! separate crates, so they verify the crate's external surface is usable.
//!
//! Planned suites (each becomes a file):
//!
//! - `deterministic.rs`     — same seed always yields the same city.
//! - `zone_transitions.rs`  — fuzzy border blending continuity.
//! - `cache_eviction.rs`    — bounded memory under infinite exploration.
//! - `ffi_basic.rs`         — exercise the C FFI surface / binary format.

// TODO(Milestone 3+): add integration test suites.
