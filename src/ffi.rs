//! # ffi.rs
//!
//! C FFI entry points for the Urbix engine.
//!
//! This module exposes the engine to any language with C interop (C, C++,
//! C#, Rust-adjacent engines, Unity via `unsafe extern`, Godot via
//! GDExtension, WebAssembly, Python via ctypes, ...). Every function is
//! declared `#[no_mangle] pub extern "C"` and forwards directly to the
//! `api`/`engine` layers, keeping the boundary thin.
//!
//! ## Memory contract
//!
//! - Functions returning allocated buffers hand ownership to the caller, who
//!   must release them via the matching `*_free` function.
//! - The `WorldEngine` handle is opaque (`*mut WorldEngine`); callers never
//!   inspect it.
//!
//! ## Planned surface
//!
//! - `urbix_engine_create(seed)` / `urbix_engine_destroy(engine)`
//! - `urbix_generate_chunk(engine, cx, cy)` + `urbix_chunk_free(buffer)`
//! - `urbix_get_zone(engine, wx, wz)`
//! - `urbix_set_draw_distance(...)` / `urbix_set_chunk_size(...)`
//!
//! The header `include/urbix.h` is generated from these declarations via
//! `cbindgen` (wired in `build.rs`, Milestone 5).

// TODO(Milestone 5): implement extern "C" functions and wire cbindgen.
