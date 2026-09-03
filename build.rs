//! build.rs
//!
//! Runs `cbindgen` to regenerate `include/urbix.h` from `src/ffi.rs` and
//! `src/data.rs` whenever the source or config changes.

fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=src/data.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let config =
        cbindgen::Config::from_file("cbindgen.toml").expect("Failed to read cbindgen.toml");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed to generate C header")
        .write_to_file("include/urbix.h");
}
