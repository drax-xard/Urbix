//! # lib.rs
//!
//! Crate root for the Urbix procedural city engine.
//!
//! This file is the public entry point of the library. It re-exports the
//! public API types so that consumers can depend on the crate without
//! reaching into internal module paths. It also wires together the FFI
//! surface exposed to foreign-language consumers (C, C++, Python, Unity,
//! Godot, etc.) via `#[no_mangle] extern "C"` functions.
//!
//! ## Architecture
//!
//! The engine is split into focused modules that mirror the layers
//! described in `Urbix_Project.md`:
//!
//! - `config`  — tunable world parameters (seed, chunk size, draw distance).
//! - `data`    — the `repr(C)` data types exchanged over the wire.
//! - `hash`    — deterministic seeded hashing used throughout generation.
//! - `zones`   — zone types, per-zone parameters, and color palettes.
//! - `region`  — the Voronoi layout of districts with fuzzy borders.
//! - `chunk`   — orchestration of per-chunk cell generation.
//! - `street`  — street grid and block subdivision.
//! - `building`— building footprint, height, and palette assignment.
//! - `interior`— interior id computation and the (stub) interior hook surface.
//! - `cache`   — the LRU chunk cache with distance-based eviction.
//! - `engine`  — the `WorldEngine` stateful facade over the above.
//! - `ffi`     — the C FFI entry points.
//!
//! ## Usage
//!
//! Most consumers will use the engine through `WorldEngine`:
//!
//! ```ignore
//! let mut engine = urbix::engine::WorldEngine::new(445566);
//! let chunk = engine.generate_chunk(0, 0);
//! ```
//!
//! Foreign-language consumers interact through the FFI functions declared
//! in the `ffi` module and the generated `include/urbix.h` header.

pub mod api;
pub mod building;
pub mod cache;
pub mod chunk;
pub mod config;
pub mod data;
pub mod engine;
pub mod ffi;
pub mod hash;
pub mod interior;
pub mod region;
pub mod street;
pub mod zones;
