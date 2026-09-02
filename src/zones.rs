//! # zones.rs
//!
//! Zone definitions and per-zone parameters for the Urbix engine.
//!
//! A convincing, varied skyline needs distinct urban regions, each with its
//! own density, height distribution, and color scheme. This module defines the
//! five zone types and the parameters that shape each one.
//!
//! ## Zone types
//!
//! - **Downtown / Business** — dense, tall skyscrapers.
//! - **Residential** — tranquil, low-rise, tree-lined.
//! - **Commercial** — busy mid-rise with bright/neon palettes.
//! - **Industrial** — grimy, wide low warehouses, open lots.
//! - **Park / Green** — low or zero buildings, foliage, open ground.
//!
//! ## Responsibilities
//!
//! - `ZoneType` enum (5 variants).
//! - `ZoneParams` struct: height range, density, block size, facade palette.
//! - Blending: `zone_params(affinity: &[f32; 5]) -> ZoneParams`, interpolating
//!   parameters from a fuzzy affinity vector produced by `region.rs`.

// TODO(Milestone 1/2): define ZoneType, ZoneParams, and the blend function.
