//! build.rs
//!
//! Regenerates `include/urbix.h` from `src/ffi.rs`/`src/data.rs` via `cbindgen`.
//! The checked-in header is always usable; generation is best-effort so offline
//! or vendored builds without network access still succeed (emits a
//! `cargo:warning` instead of failing).

use std::path::Path;

fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let crate_path = Path::new(&crate_dir);

    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=src/data.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let config_path = crate_path.join("cbindgen.toml");
    let header_path = crate_path.join("include/urbix.h");

    let config = match cbindgen::Config::from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            println!("cargo:warning=cbindgen config not readable ({e}); keeping existing include/urbix.h");
            return;
        }
    };

    let bindings = match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(b) => b,
        Err(e) => {
            println!("cargo:warning=cbindgen failed ({e}); keeping existing include/urbix.h");
            return;
        }
    };

    // Render to an in-memory buffer so we can patch the trailer inside the
    // include guard (cbindgen's `trailer` lands after `#endif`, outside the
    // guard). We inject both the layout checks and the old `URBIX_FLAG_*`
    // compat shims before the closing `#endif`.
    let mut buf = Vec::new();
    bindings.write(&mut buf);

    let mut content = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            println!("cargo:warning=cbindgen output not UTF-8 ({e}); keeping existing header");
            return;
        }
    };

    // cbindgen emits `#endif  /* URBIX_H */` as the final line for the guard.
    // Insert our additions immediately before it so they are inside the guard.
    const INJECT: &str =concat!(
        "\n/* ---- Compatibility shims for old manual header ---- */\n",
        "#define URBIX_FLAG_STREET CellFlags_IS_STREET\n",
        "#define URBIX_FLAG_PARK CellFlags_IS_PARK\n",
        "\n/* ---- Compile-time layout checks (inside include guard) ---- */\n",
        "_Static_assert(sizeof(UrbixChunkHeader) == 32, \"UrbixChunkHeader must be 32 bytes\");\n",
        "_Static_assert(sizeof(UrbixCell) == 40,      \"UrbixCell must be 40 bytes\");\n",
        "_Static_assert(_Alignof(UrbixChunkHeader) == 8, \"UrbixChunkHeader must be 8-byte aligned\");\n",
        "_Static_assert(_Alignof(UrbixCell) == 8,       \"UrbixCell must be 8-byte aligned\");\n",
    );

    if let Some(pos) = content.rfind("#endif") {
        // `pos` is the start of the final `#endif`; keep the guard line and
        // everything after it (should be just the guard comment/newline).
        let (before, after) = content.split_at(pos);
        // Avoid double-injection on re-runs: if `URBIX_FLAG_STREET` already
        // present in `before`, do not inject again.
        if !before.contains("URBIX_FLAG_STREET") {
            content = format!("{}{}{}", before.trim_end(), INJECT, after);
        }
    } else {
        println!("cargo:warning=generated header missing #endif guard; writing as-is");
    }

    if let Err(e) = std::fs::write(&header_path, content) {
        println!(
            "cargo:warning=failed to write {} ({e})",
            header_path.display()
        );
    }
}
