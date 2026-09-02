//! # main.rs
//!
//! Command-line entry point for the Urbix engine.
//!
//! This binary provides a thin CLI wrapper around the `WorldEngine`, letting
//! a user generate and dump city chunks from the shell. It is the manual
//! counterpart to the library/FFI API: it drives the same code path but with
//! human-readable flags and file output.
//!
//! ## CLI usage
//!
//! ```text
//! urbix --seed 12345 --cx 0 --cy 0 --radius 4 --format bin
//! ```
//!
//! Flags (planned for Milestone 7):
//!
//! - `--seed`        world seed (default 0)
//! - `--cx` / `--cy` chunk coordinates to generate
//! - `--radius`      number of chunks around the center
//! - `--chunk-size`  overrides the configured chunk size
//! - `--format`      output format: `bin` (default) or `json`
//!
//! ## Note
//!
//! The argument parsing is finalised in Milestone 7; until then this file
//! only declares the entry point and delegates to the engine.

fn main() {
    // TODO(Milestone 7): parse CLI flags with `clap` and drive `WorldEngine`.
    println!("Urbix engine — CLI to be implemented in Milestone 7");
}
