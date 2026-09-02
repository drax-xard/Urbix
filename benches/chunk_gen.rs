//! Benchmarks for the Urbix engine.
//!
//! This directory contains criterion-based benchmarks that measure the
//! generation pipeline's real-time performance and give clear baselines for
//! the stated performance objectives.
//!
//! Planned benchmark files:
//!
//! - `chunk_gen.rs`    — single chunk generation time.
//! - `sweep.rs`        — generating an N×N grid of chunks.
//! - `cache.rs`        — cache hit vs. miss cost.
//! - `zone_query.rs`   — cost of continuous zone/affinity queries.
//!
//! Criterion is added as a dev-dependency when the first benchmark lands
//! (Milestone 7).

// TODO(Milestone 7): add criterion benchmarks.

fn main() {
    // Placeholder: real benchmarks land with Milestone 7 (criterion dev-dep).
}
