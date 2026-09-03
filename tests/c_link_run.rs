//! End-to-end C link-and-run test for Milestone 5.
//!
//! Builds `examples/basic_usage.c` against the cbindgen-generated
//! `include/urbix.h`, links it against the crate's `staticlib`, and executes it.
//! This proves the whole FFI surface is usable from a real C consumer that
//! reaches across the ABI, not just from Rust calling itself.
//!
//! Requires the `staticlib` crate-type (declared in `Cargo.toml`).

use std::path::PathBuf;
use std::process::Command;

/// Directory containing `Cargo.toml`.
const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn staticlib_path() -> PathBuf {
    // Cargo exposes the current build profile (debug/release). The staticlib is
    // produced alongside the rlib in the same profile's target directory.
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        PathBuf::from(MANIFEST_DIR)
            .join("target")
            .to_str()
            .unwrap()
            .to_string()
    });
    PathBuf::from(target_dir)
        .join(profile)
        .join(if cfg!(target_os = "windows") {
            "urbix.lib"
        } else {
            "liburbix.a"
        })
}

#[test]
fn c_consumer_compiles_links_and_runs() {
    let lib = staticlib_path();
    assert!(
        lib.exists(),
        "staticlib not found at {} — build with a release/debug profile (crate-type=staticlib)",
        lib.display()
    );

    // 1. Compile the C consumer object.
    let include = PathBuf::from(MANIFEST_DIR).join("include");
    let src = PathBuf::from(MANIFEST_DIR)
        .join("examples")
        .join("basic_usage.c");
    let obj = std::env::temp_dir().join(format!("urbix_basic_usage_{}.o", std::process::id()));
    // 2. Linked executable.
    let bin = std::env::temp_dir().join(format!("urbix_basic_usage_{}", std::process::id()));

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());

    let status = Command::new(&cc)
        .arg("-I")
        .arg(&include)
        .arg("-c")
        .arg(&src)
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("failed to run C compiler");
    assert!(status.success(), "C compilation of basic_usage.c failed");

    // 2. Link against the Rust staticlib.
    let mut link = Command::new(&cc);
    link.arg(&obj)
        .arg("-L")
        .arg(lib.parent().unwrap())
        .arg("-lurbix")
        .arg("-o")
        .arg(&bin);
    // Rust std on macOS pulls in system frameworks via the rust driver; when
    // linking from plain `cc` we must supply them explicitly.
    #[cfg(target_os = "macos")]
    link.args(["-framework", "Security", "-framework", "CoreFoundation"]);
    #[cfg(target_os = "windows")]
    link.args(["-lws2_32", "-luserenv"]);

    let status = link.status().expect("failed to run linker");
    assert!(
        status.success(),
        "linking basic_usage.c against liburbix failed"
    );

    // 3. Run it.
    let output = Command::new(&bin)
        .output()
        .expect("failed to run linked binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "linked C consumer exited {}\nstdout: {}\nstderr: {}",
        output.status,
        stdout,
        stderr
    );
    assert!(
        stdout.trim().contains("basic_usage: ok"),
        "unexpected output: {stdout:?} {stderr:?}"
    );

    let _ = std::fs::remove_file(&obj);
    let _ = std::fs::remove_file(&bin);
}
