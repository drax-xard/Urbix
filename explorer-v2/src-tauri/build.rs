//! Build script for the Tauri shell.
//!
//! Only wires up `tauri-build` (codegen for the app manifest / ACL). Unlike the
//! engine's own `build.rs` this never touches `include/urbix.h`; the explorer
//! talks to the engine through the Rust API, not the C ABI.

fn main() {
    tauri_build::build();
}
