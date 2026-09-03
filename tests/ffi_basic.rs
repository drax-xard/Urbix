//! FFI / binary-format integration tests.
//!
//! These verify the C ABI and on-wire layout from the perspective of a foreign
//! consumer:
//!
//! - [`urbix_engine_create`] / [`urbix_generate_chunk`] / [`urbix_chunk_free`]
//!   / [`urbix_engine_destroy`] round-trip a chunk buffer with no leak of the
//!   Rust allocation.
//! - The manually-maintained `include/urbix.h` compiles as valid C against the
//!   shipped example consumer (`examples/basic_usage.c`), which asserts the
//!   expected `repr(C)` sizes and offsets (`_Static_assert`).

use std::path::PathBuf;
use std::process::Command;

use urbix::ffi::{
    urbix_chunk_free, urbix_engine_create, urbix_engine_destroy, urbix_generate_chunk,
};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn ffi_chunk_round_trip_and_layout() {
    // SAFETY: engine created and destroyed here; buffer freed once.
    let engine = urbix_engine_create(445566);
    assert!(!engine.is_null());

    // SAFETY: valid live engine.
    let buf = unsafe { urbix_generate_chunk(engine, 1, 2) };
    assert!(!buf.data.is_null());
    assert!(buf.len > 0);

    // Read the 32-byte header directly off the wire (no Rust struct involved),
    // mirroring how C would consume the buffer.
    // SAFETY: buf.data is a valid owned buffer of buf.len bytes.
    let raw = unsafe { std::slice::from_raw_parts(buf.data, buf.len as usize) };
    assert!(raw.len() >= 32, "buffer shorter than a header");
    // SAFETY: header is the first 32 bytes and is in-bounds.
    let hdr = unsafe { std::ptr::read_unaligned(buf.data.cast::<urbix::data::ChunkHeader>()) };
    assert_eq!((hdr.cx, hdr.cy), (1, 2));
    assert_eq!(hdr.cell_count, 32 * 32);

    // Buffer length must exactly equal header + cell_count cells.
    let expected = 32 + hdr.cell_count as usize * 40;
    assert_eq!(buf.len as usize, expected, "on-wire length mismatch");

    // SAFETY: buffer released once; then engine destroyed once.
    unsafe { urbix_chunk_free(buf) };
    urbix_engine_destroy(engine);
}

#[test]
fn header_compiles_as_valid_c_via_cc() {
    // Compile the shipped C consumer (which _Static_asserts the layout) against
    // the hand-maintained header. This catches header/C ABI drift without
    // needing a staticlib link step (deferred to Milestone 5).
    let include = PathBuf::from(MANIFEST_DIR).join("include");
    let src = PathBuf::from(MANIFEST_DIR)
        .join("examples")
        .join("basic_usage.c");
    let out = std::env::temp_dir().join(format!("urbix_basic_usage_{}.o", std::process::id()));

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let status = Command::new(&cc)
        .arg("-I")
        .arg(&include)
        .arg("-c")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("failed to invoke C compiler; is `cc` installed?");

    let _ = std::fs::remove_file(&out);
    assert!(
        status.success(),
        "C consumer failed to compile against include/urbix.h"
    );
}
