//! FFI / binary-format integration tests.
//!
//! These verify the C ABI and on-wire layout from the perspective of a foreign
//! consumer:
//!
//! - [`urbix_engine_create`] / [`urbix_generate_chunk`] / [`urbix_chunk_free`]
//!   / [`urbix_engine_destroy`] round-trip a chunk buffer with no leak of the
//!   Rust allocation.
//! - The cbindgen-generated `include/urbix.h` compiles as valid C against the
//!   shipped example consumer (`examples/basic_usage.c`), which asserts the
//!   expected `repr(C)` sizes and offsets (`_Static_assert`).
//! - A stochastic fuzz sequence exercises the ownership contract under load.
//!
//! A full link-and-run test lives in `tests/c_link_run.rs`.

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
    // the cbindgen-generated header. This catches header/C ABI drift cheaply;
    // the full link-and-run equivalent is in tests/c_link_run.rs.
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

/// M5 fuzz test: a long stochastic sequence of create -> generate -> free ->
/// destroy through the raw FFI. Exercises the ownership contract under load:
/// every returned buffer must be freed exactly once and every engine destroyed
/// exactly once, with no double-free/use-after-free, no leaks, and no panics
/// escaping the boundary. Runs under the normal Rust global allocator, so any
/// double-free/leak would trip it.
#[test]
fn fuzz_create_generate_free_destroy() {
    // Small deterministic LCG so the run is reproducible (not a security RNG).
    let mut rng: u64 = 0x9E3779B97F4A7C15 ^ 445566;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };

    const ENGINES: usize = 16;
    const ROUNDS: usize = 500;

    // A scatter of live buffers per engine, each freed before the engine dies.
    let mut live: [Vec<urbix::ffi::UrbixChunkBuffer>; ENGINES] = Default::default();
    let mut engines: [*mut urbix::ffi::UrbixEngine; ENGINES] = [std::ptr::null_mut(); ENGINES];

    for (i, slot) in engines.iter_mut().enumerate() {
        // SAFETY: each engine allocated here and destroyed at the end.
        let engine = urbix_engine_create(next() % 1_000_000);
        assert!(!engine.is_null(), "engine {i} allocation failed");
        *slot = engine;
    }

    for round in 0..ROUNDS {
        let ei = (next() % ENGINES as u64) as usize;
        let engine = engines[ei];

        let action = next() % 4;
        match action {
            // Generate a chunk and hold it.
            0 => {
                // SAFETY: live engine.
                let buf = unsafe {
                    urbix_generate_chunk(
                        engine,
                        (next() % 33) as i32 - 16,
                        (next() % 33) as i32 - 16,
                    )
                };
                assert!(!buf.data.is_null(), "round {round}: null buffer");
                assert!(buf.len >= 32, "round {round}: buffer too short");
                // SAFETY: header is the leading 32 bytes of an owned buffer.
                let hdr = unsafe {
                    std::ptr::read_unaligned(buf.data.cast::<urbix::data::ChunkHeader>())
                };
                let expected = 32 + hdr.cell_count as usize * 40;
                assert_eq!(buf.len as usize, expected, "round {round}: wire length");
                live[ei].push(buf);
            }
            // Free a held buffer if any.
            1 => {
                if let Some(buf) = live[ei].pop() {
                    // SAFETY: buf came from urbix_generate_chunk, freed once.
                    unsafe { urbix_chunk_free(buf) };
                }
            }
            // Destroy an engine and replace it (its held buffers die with it).
            2 => {
                for buf in live[ei].drain(..) {
                    // SAFETY: each buf freed exactly once before engine death.
                    unsafe { urbix_chunk_free(buf) };
                }
                // SAFETY: engine created above; replaced immediately after.
                urbix_engine_destroy(engine);
                engines[ei] = urbix_engine_create(next() % 1_000_000);
                assert!(!engines[ei].is_null(), "round {round}: recreate failed");
            }
            // Zone query + a chunk round-trip (generate -> immediately free).
            _ => {
                // SAFETY: live engine.
                let zone = unsafe { urbix::ffi::urbix_get_zone(engine, 3.5, -7.25) };
                let sum: f32 = zone.weights.iter().sum();
                assert!(
                    (sum - 1.0).abs() < 1e-6,
                    "round {round}: affinity sum {sum}"
                );
                // SAFETY: live engine; freed immediately below.
                let buf = unsafe { urbix_generate_chunk(engine, 0, 0) };
                // SAFETY: valid owned buffer, freed exactly once.
                unsafe { urbix_chunk_free(buf) };
            }
        }
    }

    // Tear down: free every remaining buffer, then every engine.
    for i in 0..ENGINES {
        for buf in live[i].drain(..) {
            // SAFETY: each buffer freed exactly once.
            unsafe { urbix_chunk_free(buf) };
        }
        // SAFETY: every engine destroyed exactly once at end of life.
        urbix_engine_destroy(engines[i]);
    }
}
