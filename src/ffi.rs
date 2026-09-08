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
//! - [`urbix_generate_interior`] returns a [`UrbixInterior`] in the same
//!   pattern: `data` is a Rust-allocated payload the caller owns and must
//!   release with [`urbix_interior_free`].
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

/// A generated building interior handed to foreign code.
///
/// `data` points at `len` bytes laid out as one payload chunk per floor, in
/// storey order:
///
/// ```text
/// floor i (i in 0..floor_count):
///   tiles[footprint_w * footprint_d]   // one Tile byte each, row-major
///   kinds[footprint_w * footprint_d]   // room-kind tag byte each, 0 = not room
/// ```
///
/// Every floor shares the same `footprint_w × footprint_d` grid, so `len` is
/// exactly `floor_count * 2 * footprint_w * footprint_d`. `Tile` bytes are the
/// [`crate::layout::Tile`] enum values (0 = void, 1 = wall, 2 = door, 3 = core,
/// 4 = corridor, 5 = room). The caller owns this payload and must release it
/// with [`urbix_interior_free`]. An unbuilt cell (height ≤ 0) or null engine
/// yields a zeroed header with `data == null` and `len == 0`.
#[repr(C)]
pub struct UrbixInterior {
    /// Stable interior key (the built cell's deterministic key).
    pub interior_id: u64,
    /// World seed used during generation.
    pub seed: u64,
    /// Dominant [`crate::zones::ZoneType`] index (0–4).
    pub zone: u8,
    /// [`crate::layout::DoorSide`] index (0 = west, 1 = east, 2 = north, 3 = south).
    pub door_side: u8,
    /// Shared floor-grid width in tiles.
    pub footprint_w: u8,
    /// Shared floor-grid depth in tiles.
    pub footprint_d: u8,
    /// Number of storeys (`len` = `floor_count * 2 * footprint_w * footprint_d`).
    pub floor_count: u16,
    /// Total payload byte length; 0 when unbuilt.
    pub len: u64,
    /// Start of the payload (see struct docs); null when unbuilt.
    pub data: *mut u8,
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

/// Generate the interior layout for the built cell at world space `(wx, wz)`.
///
/// The engine locates the cell's chunk from the coordinates alone (world space
/// is the canonical interior key — see [`crate::chunk::interior_context_for`]),
/// generates it if needed, and derives the layout from the same path the Rust
/// APIs use, so C consumers and Rust consumers always agree. Requires a built
/// cell (`height > 0`); otherwise the returned header is zeroed.
///
/// On success returns an owned [`UrbixInterior`]; release it with
/// [`urbix_interior_free`].
///
/// ## Safety
///
/// `engine` must be a valid, non-null handle from [`urbix_engine_create`].
#[no_mangle]
pub unsafe extern "C" fn urbix_generate_interior(
    engine: *mut UrbixEngine,
    wx: i32,
    wz: i32,
) -> UrbixInterior {
    let empty = empty_interior();
    if engine.is_null() {
        return empty;
    }
    // SAFETY: caller guarantees a valid non-concurrently-used engine handle.
    let engine = unsafe { &mut *(engine as *mut WorldEngine) };

    let n = i64::from(engine.config().chunk_size);
    let cx = i64::from(wx).div_euclid(n) as i32;
    let cy = i64::from(wz).div_euclid(n) as i32;
    let chunk = engine.generate_chunk(cx, cy);

    let lx = i64::from(wx).rem_euclid(n) as usize;
    let ly = i64::from(wz).rem_euclid(n) as usize;
    let cell = chunk.get_cell(ly * n as usize + lx);
    if cell.height <= 0.0 {
        return empty;
    }

    let config = engine.config();
    let ctx = crate::chunk::interior_context_for(config, i64::from(wx), i64::from(wz), &cell);
    let blueprint = config.blueprint_for(ctx.zone);
    let layout = crate::interior::generate_layout(cell.interior_id, &ctx, &blueprint);

    let first = &layout.floors[0];
    let payload = interior_payload(&layout);
    let len = payload.len() as u64;
    let data = Box::into_raw(payload.into_boxed_slice()) as *mut u8;
    UrbixInterior {
        interior_id: layout.id,
        seed: layout.seed,
        zone: ctx.zone as u8,
        door_side: ctx.door_side as u8,
        footprint_w: first.width,
        footprint_d: first.depth,
        floor_count: layout.floors.len() as u16,
        len,
        data,
    }
}

/// Release an interior buffer returned by [`urbix_generate_interior`].
///
/// ## Safety
///
/// `interior` must be an *unreleased* result from [`urbix_generate_interior`].
/// Calling this twice on the same buffer (or with an unrelated buffer) is
/// double-free / undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn urbix_interior_free(interior: UrbixInterior) {
    if interior.data.is_null() {
        return;
    }
    // SAFETY: caller guarantees interior came from urbix_generate_interior (or
    // a null verified above), so data is a Rust-allocated boxed slice.
    drop(unsafe {
        Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            interior.data,
            interior.len as usize,
        ))
    });
}

/// An unbuilt ("empty") interior: zeroed header, no payload.
fn empty_interior() -> UrbixInterior {
    UrbixInterior {
        interior_id: 0,
        seed: 0,
        zone: 0,
        door_side: 0,
        footprint_w: 0,
        footprint_d: 0,
        floor_count: 0,
        len: 0,
        data: ptr::null_mut(),
    }
}

/// Pack a layout's floors into the FFI payload layout.
///
/// Per floor, row-major tiles then room-kind bytes:
/// `floor_count * 2 * footprint_w * footprint_d` bytes total.
fn interior_payload(layout: &crate::layout::InteriorLayout) -> Vec<u8> {
    let cell_bytes = layout
        .floors
        .iter()
        .map(|f| usize::from(f.width) * usize::from(f.depth))
        .sum::<usize>();
    let mut out = Vec::with_capacity(cell_bytes * 2);
    for floor in &layout.floors {
        out.extend(floor.tiles.iter().map(|t| *t as u8));
        out.extend(floor.kinds.iter().copied());
    }
    out
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
    use std::slice;

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

    #[test]
    fn interior_round_trips_and_is_deterministic() {
        // Engine created, used single-threaded, destroyed here.
        let engine = urbix_engine_create(445566);
        assert!(!engine.is_null());

        // SAFETY: live engine.
        let buf = unsafe { urbix_generate_chunk(engine, 0, 0) };
        assert!(!buf.data.is_null());
        // SAFETY: valid buffer; header is in-bounds.
        let header =
            unsafe { std::ptr::read_unaligned(buf.data.cast::<crate::data::ChunkHeader>()) };
        let mut built = None;
        let mut street = None;
        for i in 0..header.cell_count as usize {
            // SAFETY: cell i lies within buf.len (header + cell_count * 40).
            let cell = unsafe {
                std::ptr::read_unaligned(buf.data.add(32 + i * 40).cast::<crate::data::Cell>())
            };
            let lx = (i % usize::from(header.chunk_size)) as i32;
            let ly = (i / usize::from(header.chunk_size)) as i32;
            if cell.height > 0.0 && built.is_none() {
                built = Some((lx, ly));
            }
            if cell.height == 0.0 && street.is_none() {
                street = Some((lx, ly));
            }
            if built.is_some() && street.is_some() {
                break;
            }
        }
        // SAFETY: scan buffer released once.
        unsafe { urbix_chunk_free(buf) };

        let (wx, wz) = built.expect("seed 445566 has a built cell near (0,0)");
        // SAFETY: live engine.
        let i1 = unsafe { urbix_generate_interior(engine, wx, wz) };
        assert!(!i1.data.is_null());
        assert!(i1.door_side <= 3, "door_side is a valid DoorSide");
        assert!(i1.floor_count >= 1);
        assert!(
            i1.footprint_w >= 7 && i1.footprint_d >= 7,
            "footprint floored at 7"
        );
        let expected =
            u64::from(i1.floor_count) * 2 * u64::from(i1.footprint_w) * u64::from(i1.footprint_d);
        assert_eq!(i1.len, expected);

        // SAFETY: payload is exactly i1.len bytes.
        let payload1 = unsafe { slice::from_raw_parts(i1.data, i1.len as usize) };
        let grid = usize::from(i1.footprint_w) * usize::from(i1.footprint_d);
        let tiles = &payload1[..grid];
        assert!(
            tiles.iter().all(|&b| b <= 5),
            "tile bytes within enum range"
        );
        assert!(tiles.contains(&1), "has exterior walls");
        assert!(tiles.contains(&3), "has circulation core");

        // Determinism: a second call yields the identical payload.
        // SAFETY: live engine.
        let i2 = unsafe { urbix_generate_interior(engine, wx, wz) };
        assert_eq!(i2.interior_id, i1.interior_id);
        assert_eq!(i2.seed, i1.seed);
        assert_eq!(i2.len, i1.len);
        // SAFETY: both payloads are valid.
        assert_eq!(
            unsafe { slice::from_raw_parts(i2.data, i2.len as usize) },
            payload1
        );

        // SAFETY: each interior released once; engine destroyed once.
        unsafe {
            urbix_interior_free(i1);
            urbix_interior_free(i2);
        }
        urbix_engine_destroy(engine);
    }

    #[test]
    fn interior_negative_world_coords_round_trip() {
        // Engine created/destroyed here.
        let engine = urbix_engine_create(445566);
        assert!(!engine.is_null());
        // SAFETY: live engine; div_euclid handles negatives.
        let a = unsafe { urbix_generate_interior(engine, -3, -7) };
        let b = unsafe { urbix_generate_interior(engine, -3, -7) };
        assert_eq!(a.interior_id, b.interior_id);
        assert_eq!(a.len, b.len);
        if !a.data.is_null() {
            // SAFETY: payloads are valid and equal length.
            assert_eq!(
                unsafe { slice::from_raw_parts(a.data, a.len as usize) },
                unsafe { slice::from_raw_parts(b.data, b.len as usize) }
            );
        }
        // SAFETY: each interior released once; engine destroyed once.
        unsafe {
            urbix_interior_free(a);
            urbix_interior_free(b);
        }
        urbix_engine_destroy(engine);
    }

    #[test]
    fn unbuilt_cell_yields_empty_interior() {
        // Engine created/destroyed here.
        let engine = urbix_engine_create(7);
        assert!(!engine.is_null());

        // SAFETY: live engine.
        let buf = unsafe { urbix_generate_chunk(engine, 0, 0) };
        // SAFETY: valid buffer.
        let header =
            unsafe { std::ptr::read_unaligned(buf.data.cast::<crate::data::ChunkHeader>()) };
        let mut street = None;
        for i in 0..header.cell_count as usize {
            // SAFETY: cell i lies within buf.len.
            let cell = unsafe {
                std::ptr::read_unaligned(buf.data.add(32 + i * 40).cast::<crate::data::Cell>())
            };
            if cell.height == 0.0 {
                let lx = (i % usize::from(header.chunk_size)) as i32;
                let ly = (i / usize::from(header.chunk_size)) as i32;
                street = Some((lx, ly));
                break;
            }
        }
        // SAFETY: scan buffer released once.
        unsafe { urbix_chunk_free(buf) };

        let (wx, wz) = street.expect("every chunk has street cells");
        // SAFETY: live engine; unbuilt cells yield a zeroed interior.
        let interior = unsafe { urbix_generate_interior(engine, wx, wz) };
        assert!(interior.data.is_null());
        assert_eq!(interior.len, 0);
        assert_eq!(interior.floor_count, 0);
        // SAFETY: null payload is a safe no-op.
        unsafe { urbix_interior_free(interior) };
        urbix_engine_destroy(engine);
    }

    #[test]
    fn interior_null_engine_safe() {
        // SAFETY: null engine is a no-op, not UB.
        let interior = unsafe { urbix_generate_interior(ptr::null_mut(), 10, 10) };
        assert!(interior.data.is_null());
        assert_eq!(interior.len, 0);
        // SAFETY: null payload is a safe no-op.
        unsafe { urbix_interior_free(interior) };
    }
}
