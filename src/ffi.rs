//! # ffi.rs
//!
//! C FFI entry points for the Urbix engine.
//!
//! This module exposes the engine to any language with C interop (C, C++,
//! C#, Rust-adjacent engines, Unity via `unsafe extern`, Godot via
//! GDExtension, WebAssembly, Python via ctypes, ...). Every function is
//! declared `#[no_mangle] pub extern "C"` and forwards directly to the
//! `engine` layer, keeping the boundary thin.
//!
//! ## Memory contract
//!
//! - [`urbix_engine_create`] allocates an opaque engine; the caller owns it and
//!   must release it with [`urbix_engine_destroy`]. The handle is never
//!   dereferenced by foreign code — it is only passed back and forth.
//! - [`urbix_generate_chunk`] returns a [`UrbixChunkBuffer`] whose `data` is a
//!   Rust-allocated byte buffer. **Ownership transfers to the caller**, who must
//!   release it with [`urbix_chunk_free`]. It must never be freed by a foreign
//!   allocator (`free`, `delete`, ...).
//! - All other functions take borrowed state and transfer no ownership.
//!
//! ## C ABI notes
//!
//! The public types (`UrbixEngine`, `UrbixChunkBuffer`, `UrbixZoneAffinity`)
//! are `#[repr(C)]` so their layout is fixed and stable; `include/urbix.h` is
//! generated from this module by `cbindgen` (`build.rs`).

use std::ptr;

use crate::config::WorldConfig;
use crate::data::ChunkBuffer;
use crate::engine::WorldEngine;
use crate::zones::ZONE_COUNT;

/// Opaque handle to a [`WorldEngine`], owned by the caller.
///
/// Never dereference from foreign code; pass it back to the FFI functions.
/// Deliberately not `#[repr(C)]`: it is a zero-sized opaque marker whose layout
/// is irrelevant, and cbindgen emits an opaque forward-declared typedef for it.
pub struct UrbixEngine {
    _private: [u8; 0],
}

/// An owned chunk's packed bytes handed to foreign code.
///
/// `data` points at `len` bytes: a [`crate::data::ChunkHeader`] followed by
/// `cell_count` [`crate::data::Cell`] records with no padding. The caller owns
/// this buffer and must free it with [`urbix_chunk_free`].
#[repr(C)]
pub struct UrbixChunkBuffer {
    /// Start of the wire buffer (header + cells).
    pub data: *mut u8,
    /// Total byte length (`sizeof(header) + cell_count * sizeof(cell)`).
    pub len: u64,
}

/// A blended zone-affinity vector, one weight per [`crate::zones::ZoneType`].
#[repr(C)]
pub struct UrbixZoneAffinity {
    /// Per-zone weights summing to ~1.0.
    pub weights: [f32; ZONE_COUNT],
}

/// Construct an engine with the given seed and default configuration.
///
/// Returns an opaque handle the caller owns and must release with
/// [`urbix_engine_destroy`]. Returns a null pointer on allocation failure.
#[no_mangle]
pub extern "C" fn urbix_engine_create(seed: u64) -> *mut UrbixEngine {
    let engine = WorldEngine::new(seed);
    Box::into_raw(Box::new(engine)) as *mut UrbixEngine
}

/// Destroy an engine previously created with [`urbix_engine_create`].
///
/// A null pointer is a no-op. After this call the handle is dangling and must
/// not be used.
#[no_mangle]
pub extern "C" fn urbix_engine_destroy(engine: *mut UrbixEngine) {
    if engine.is_null() {
        return;
    }
    // SAFETY: engine comes from urbix_engine_create (or a null verified above).
    drop(unsafe { Box::from_raw(engine as *mut WorldEngine) });
}

/// Generate (or fetch from cache) the chunk at `(cx, cy)`.
///
/// On success returns an owned [`UrbixChunkBuffer`] with `len == 0` on failure,
/// and `data == null`. The caller must release a successful buffer with
/// [`urbix_chunk_free`].
///
/// ## Safety
///
/// `engine` must be a valid, non-null handle from [`urbix_engine_create`].
#[no_mangle]
pub unsafe extern "C" fn urbix_generate_chunk(
    engine: *mut UrbixEngine,
    cx: i32,
    cy: i32,
) -> UrbixChunkBuffer {
    let mut empty = UrbixChunkBuffer {
        data: ptr::null_mut(),
        len: 0,
    };
    if engine.is_null() {
        return empty;
    }
    // SAFETY: caller guarantees a valid non-concurrently-used engine handle.
    let engine = unsafe { &mut *(engine as *mut WorldEngine) };
    let chunk: ChunkBuffer = engine.generate_chunk(cx, cy);
    let (data, len) = chunk.into_raw_bytes();
    empty.data = data;
    empty.len = len as u64;
    empty
}

/// Release a chunk buffer returned by [`urbix_generate_chunk`].
///
/// ## Safety
///
/// `buf` must be an *unreleased* buffer from [`urbix_generate_chunk`]. Calling
/// this twice on the same buffer (or with an unrelated buffer) is
/// double-free / undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn urbix_chunk_free(buf: UrbixChunkBuffer) {
    if buf.data.is_null() {
        return;
    }
    // SAFETY: caller guarantees buf came from urbix_generate_chunk (or a null
    // verified above).
    drop(unsafe { ChunkBuffer::from_raw_bytes(buf.data, buf.len as usize) });
}

/// Query the continuous zone-affinity vector at world coordinates.
///
/// ## Safety
///
/// `engine` must be a valid, non-null handle from [`urbix_engine_create`].
#[no_mangle]
pub unsafe extern "C" fn urbix_get_zone(
    engine: *mut UrbixEngine,
    wx: f64,
    wz: f64,
) -> UrbixZoneAffinity {
    if engine.is_null() {
        return UrbixZoneAffinity {
            weights: [0.0; ZONE_COUNT],
        };
    }
    // SAFETY: caller guarantees a valid, non-concurrently-used handle.
    let engine = unsafe { &*(engine as *const WorldEngine) };
    UrbixZoneAffinity {
        weights: engine.get_zone_affinity(wx, wz),
    }
}

/// Set the draw distance (in chunk Chebyshev units).
///
/// ## Safety
///
/// `engine` must be a valid, non-null handle from [`urbix_engine_create`].
#[no_mangle]
pub unsafe extern "C" fn urbix_set_draw_distance(engine: *mut UrbixEngine, radius: u32) {
    if engine.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid, non-concurrently-used handle.
    let engine = unsafe { &mut *(engine as *mut WorldEngine) };
    engine.set_draw_distance(radius);
}

/// Set the cells-per-side chunk size for subsequent generation.
///
/// Existing cached chunks are cleared (they were built at the old size).
/// Passing `0` is a no-op rather than a panic, since the C boundary must not
/// unwind into the caller.
///
/// ## Safety
///
/// `engine` must be a valid, non-null handle from [`urbix_engine_create`].
#[no_mangle]
pub unsafe extern "C" fn urbix_set_chunk_size(engine: *mut UrbixEngine, size: u16) {
    if engine.is_null() || size == 0 {
        return;
    }
    // SAFETY: caller guarantees a valid, non-concurrently-used handle.
    let engine = unsafe { &mut *(engine as *mut WorldEngine) };
    engine.set_chunk_size(size);
}

/// Construct an engine from a fully-specified [`WorldConfig`].
///
/// Returns null if `config` is null or `!config.is_valid()`.
///
/// ## Safety
///
/// `config` must be a valid pointer to a `WorldConfig`, or null.
#[no_mangle]
pub unsafe extern "C" fn urbix_engine_create_with_config(
    config: *const WorldConfig,
) -> *mut UrbixEngine {
    if config.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller guarantees `config` points to a valid WorldConfig.
    let cfg = unsafe { std::ptr::read(config) };
    if !cfg.is_valid() {
        return ptr::null_mut();
    }
    let engine = WorldEngine::with_config(cfg);
    Box::into_raw(Box::new(engine)) as *mut UrbixEngine
}

/// Replace an engine's configuration wholesale (modular customization).
///
/// Regenerates the Voronoi diagram and clears the chunk cache. No-ops on null
/// handles or invalid configs; never panics across the FFI boundary.
///
/// ## Safety
///
/// `engine` must be a valid handle, `config` a valid `WorldConfig` pointer.
#[no_mangle]
pub unsafe extern "C" fn urbix_set_config(engine: *mut UrbixEngine, config: *const WorldConfig) {
    if engine.is_null() || config.is_null() {
        return;
    }
    // SAFETY: caller guarantees both pointers are valid.
    let cfg = unsafe { std::ptr::read(config) };
    if !cfg.is_valid() {
        return;
    }
    let engine = unsafe { &mut *(engine as *mut WorldEngine) };
    engine.set_config(cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_destroy_null_safe() {
        // urbix_engine_destroy tolerates null.
        urbix_engine_destroy(ptr::null_mut());
        // Null generate/free are safe no-ops.
        // SAFETY: null engine.
        let buf = unsafe { urbix_generate_chunk(ptr::null_mut(), 0, 0) };
        assert_eq!(buf.len, 0);
        assert!(buf.data.is_null());
        // SAFETY: null buffer.
        unsafe { urbix_chunk_free(buf) };
    }

    #[test]
    fn generate_and_free_round_trips() {
        // Engine created here, used single-threaded, destroyed here.
        let engine = urbix_engine_create(445566);
        assert!(!engine.is_null());

        // SAFETY: valid live engine.
        let buf = unsafe { urbix_generate_chunk(engine, 1, 2) };
        assert!(!buf.data.is_null());
        // SAFETY: valid buffer from urbix_generate_chunk, header is in-bounds.
        let header =
            unsafe { std::ptr::read_unaligned(buf.data.cast::<crate::data::ChunkHeader>()) };
        assert_eq!(header.cx, 1);
        assert_eq!(header.cy, 2);
        assert_eq!(header.chunk_size, 32);
        assert_eq!(header.cell_count, 32 * 32);
        assert_eq!(buf.len, 32 + header.cell_count as u64 * 40);

        // SAFETY: buf released once; engine destroyed once.
        unsafe { urbix_chunk_free(buf) };
        urbix_engine_destroy(engine);
    }

    #[test]
    fn zone_query_and_setters_work() {
        // Engine created and destroyed here, single-threaded.
        let engine = urbix_engine_create(7);
        assert!(!engine.is_null());

        // SAFETY: live engine.
        let zone = unsafe { urbix_get_zone(engine, 3.0, 4.0) };
        let sum: f32 = zone.weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "affinity sum={sum}");

        // SAFETY: live engine; setters are safe on a valid handle.
        unsafe {
            urbix_set_draw_distance(engine, 4);
            urbix_set_chunk_size(engine, 16);
        }
        // SAFETY: live engine.
        let buf = unsafe { urbix_generate_chunk(engine, 0, 0) };
        assert!(!buf.data.is_null());
        // SAFETY: valid buffer.
        let header =
            unsafe { std::ptr::read_unaligned(buf.data.cast::<crate::data::ChunkHeader>()) };
        assert_eq!(header.chunk_size, 16);
        assert_eq!(header.cell_count, 16 * 16);

        // SAFETY: buf released once; engine destroyed once.
        unsafe { urbix_chunk_free(buf) };
        urbix_engine_destroy(engine);
    }

    #[test]
    fn set_chunk_size_zero_is_noop_not_panic() {
        // Engine created/destroyed here.
        let engine = urbix_engine_create(42);
        assert!(!engine.is_null());
        // SAFETY: live engine; 0 must not unwind.
        unsafe { urbix_set_chunk_size(engine, 0) };
        // SAFETY: live engine.
        let buf = unsafe { urbix_generate_chunk(engine, 0, 0) };
        // SAFETY: valid buffer.
        let header =
            unsafe { std::ptr::read_unaligned(buf.data.cast::<crate::data::ChunkHeader>()) };
        assert_eq!(header.chunk_size, 32);
        // SAFETY: release buffer, then engine.
        unsafe { urbix_chunk_free(buf) };
        urbix_engine_destroy(engine);
    }
}
