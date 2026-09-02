//! # api.rs
//!
//! Public, high-level API surface of the Urbix engine.
//!
//! This module defines the ergonomic, language-agnostic facade that foreign
//! consumers interact with. It is intentionally free of Rust-specific types
//! in its signatures where possible, favouring flat `repr(C)` records so the
//! same shape is usable from C, C++, Python, and other languages.
//!
//! ## Purpose
//!
//! Keeping a dedicated `api` layer lets the internal modules (`chunk`,
//! `region`, `cache`, ...) stay focused on pure generation logic while this
//! module owns the boundary conventions: error codes, memory ownership, and
//! the binary chunk layout. The `ffi` module forwards to these same
//! functions, so the CLI, the library, and the C bindings all share one
//! implementation path.
//!
//! ## Planned surface
//!
//! - `generate_chunk(cx, cy)` → owned chunk buffer
//! - `get_zone_affinity(world_x, world_z)` → blended zone vector
//! - `set_draw_distance(radius)` / `set_chunk_size(size)`
//!
//! Concrete signatures land with Milestone 4 (engine + cache) and are
//! hardened in Milestone 5 (FFI + binary format).

// TODO(Milestone 4): add generate_chunk / get_zone_affinity / config setters.
