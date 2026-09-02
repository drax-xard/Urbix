//! # config.rs
//!
//! Global tunable parameters for the Urbix engine.
//!
//! This module centralizes every knob that shapes generation, so behaviour is
//! predictable and easy to experiment with. Configuration is captured in a
//! plain data struct (`WorldConfig`) rather than scattered constants.
//!
//! ## Fields (representative)
//!
//! - `seed`                — world seed; the same seed always yields the same city.
//! - `chunk_size`          — cells per chunk side (default 32; 16/64/128 supported).
//! - `draw_distance`       — chunk radius kept in cache before eviction.
//! - `voronoi_site_count`  — number of district sites (24–48) spread over the span.
//!
//! Being `repr(C)` and plain data, `WorldConfig` can cross the FFI boundary so
//! foreign consumers can configure the engine uniformly.

// TODO(Milestone 1): define the WorldConfig struct and fields.
