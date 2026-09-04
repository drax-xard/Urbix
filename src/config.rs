//! # config.rs
//!
//! Global tunable parameters for the Urbix engine.
//!
//! This module centralizes every knob that shapes generation, so behaviour is
//! predictable and easy to experiment with. Configuration is captured in a
//! plain data struct (`WorldConfig`) rather than scattered constants.
//!
//! ## Fields
//!
//! - `seed`               — world seed; the same seed always yields the same city.
//! - `chunk_size`         — cells per chunk side (default 32; 16/64/128 supported).
//! - `draw_distance`      — chunk Chebyshev radius kept in cache before eviction.
//! - `voronoi_site_count` — number of district sites (24–48) spread over the span.
//! - `voronoi_span`       — half-extent of Voronoi sites ([-span, span]²).
//! - `shepard_power`      — exponent `p` in `1/d^p` blending.
//! - `shepard_epsilon`    — guard for `d=0` in Shepard weighting.
//! - `zone_weights`       — weighted frequency for tagging Voronoi sites.
//! - `zones`              — per-zone `ZoneParams` (height, density, block, palette).
//! - `zone_hues`          — per-zone RGB hues for visualization.
//! - `interior_size`      — width/height range for placeholder interiors.
//!
//! Being `#[repr(C)]` and plain data, `WorldConfig` can cross the FFI boundary
//! so foreign consumers can configure the engine uniformly. `Default` provides
//! a sensible starting point for each knob. Text-file loading (`from_file`) is
//! provided for modular customization (TOML or JSON, Milestone 8).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data::InteriorId;
use crate::layout::{blueprint_defaults, default_blueprints, Blueprint, InteriorContext};
use crate::zones::{ZoneParams, ZoneType, ZONE_COUNT};

/// Serde `default` fn for [`WorldConfig::interior_floor_height`].
#[must_use]
pub const fn default_interior_floor_height() -> f32 {
    crate::layout::DEFAULT_FLOOR_HEIGHT
}

/// Serde `default` fn for [`WorldConfig::interior_max_floors`].
#[must_use]
pub const fn default_interior_max_floors() -> u8 {
    crate::layout::DEFAULT_MAX_FLOORS
}

/// Serde `default` fn for [`WorldConfig::interior_blueprints`].
#[must_use]
pub fn default_interior_blueprints() -> [Blueprint; ZONE_COUNT] {
    default_blueprints()
}

/// Default hues used by `examples/viz.rs` / `interactive.rs` (promoted to config
/// in Milestone 8 so artists tune without recompiling).
pub const DEFAULT_ZONE_HUES: [[u8; 3]; ZONE_COUNT] = [
    [100, 150, 220], // Downtown
    [96, 180, 90],   // Residential
    [235, 160, 70],  // Commercial
    [150, 130, 115], // Industrial
    [140, 205, 120], // Park
];

/// A `#[repr(C)]` snapshot of every tunable that shapes world generation.
///
/// The struct holds only plain integer/float data so it is trivially
/// FFI-safe and can be passed by pointer to and from C. Construct it with
/// [`WorldConfig::default`] and override the fields you care about, or build
/// a fresh one via `..Default::default()`. For file-based tuning, use
/// [`WorldConfig::from_file`] (TOML or JSON).
///
/// ## Example
///
/// ```
/// use urbix::config::WorldConfig;
///
/// let cfg = WorldConfig {
///     seed: 445566,
///     ..Default::default()
/// };
/// assert_eq!(cfg.seed, 445566);
/// assert!(cfg.is_valid());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct WorldConfig {
    /// World seed; the same seed always yields the same city.
    pub seed: u64,
    /// Cells per chunk side. Must be > 0; defaults to 32.
    pub chunk_size: u16,
    /// Chunk Chebyshev radius kept in cache before eviction. Must be > 0.
    pub draw_distance: u32,
    /// Number of district sites in the Voronoi map. Typically 24–48.
    pub voronoi_site_count: u16,
    /// Half-extent of Voronoi site distribution ([-span, span]²). Default 10_000.
    pub voronoi_span: f64,
    /// Shepard exponent `p` in `1/d^p`. Default 4.0.
    pub shepard_power: f64,
    /// Shepard epsilon guard for `d=0`. Default 1e-8.
    pub shepard_epsilon: f64,
    /// Weighted frequency for tagging Voronoi sites per zone. Must sum ~1.0.
    pub zone_weights: [f64; ZONE_COUNT],
    /// Per-zone parameters (height, density, block, palette).
    pub zones: [ZoneParams; ZONE_COUNT],
    /// Per-zone RGB hues for visualization (promoted from viz).
    pub zone_hues: [[u8; 3]; ZONE_COUNT],
    /// Interior room width range [min, max] (inclusive, 6..14 default).
    pub interior_width_range: [u16; 2],
    /// Interior room height range [min, max] (inclusive, 6..14 default).
    pub interior_height_range: [u16; 2],
    /// World units per storey: converts building height to floor count.
    #[serde(default = "default_interior_floor_height")]
    pub interior_floor_height: f32,
    /// Maximum number of interior floors when deriving from height.
    #[serde(default = "default_interior_max_floors")]
    pub interior_max_floors: u8,
    /// Per-zone interior layout rule tables (Milestone 9 blueprint schema).
    #[serde(default = "default_interior_blueprints")]
    pub interior_blueprints: [Blueprint; ZONE_COUNT],
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            chunk_size: 32,
            draw_distance: 8,
            voronoi_site_count: 32,
            voronoi_span: 10_000.0,
            shepard_power: 4.0,
            shepard_epsilon: 1e-8,
            zone_weights: [0.25, 0.30, 0.20, 0.15, 0.10],
            zones: [
                ZoneParams {
                    height_min: 40.0,
                    height_max: 200.0,
                    density: 0.95,
                    block_size: 4,
                    palette_count: 6,
                },
                ZoneParams {
                    height_min: 4.0,
                    height_max: 18.0,
                    density: 0.80,
                    block_size: 8,
                    palette_count: 5,
                },
                ZoneParams {
                    height_min: 12.0,
                    height_max: 60.0,
                    density: 0.90,
                    block_size: 5,
                    palette_count: 7,
                },
                ZoneParams {
                    height_min: 6.0,
                    height_max: 25.0,
                    density: 0.70,
                    block_size: 12,
                    palette_count: 4,
                },
                ZoneParams {
                    height_min: 0.0,
                    height_max: 2.0,
                    density: 0.10,
                    block_size: 16,
                    palette_count: 3,
                },
            ],
            zone_hues: DEFAULT_ZONE_HUES,
            interior_width_range: [6, 14],
            interior_height_range: [6, 14],
            interior_floor_height: crate::layout::DEFAULT_FLOOR_HEIGHT,
            interior_max_floors: crate::layout::DEFAULT_MAX_FLOORS,
            interior_blueprints: crate::layout::default_blueprints(),
        }
    }
}

impl WorldConfig {
    /// Validate the invariant-satisfying constraints on every field.
    ///
    /// Returns `false` if any field is out of its supported range, so
    /// callers (engine construction, FFI setters) can reject bad config early.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::config::WorldConfig;
    ///
    /// assert!(WorldConfig::default().is_valid());
    /// let bad = WorldConfig { chunk_size: 0, ..Default::default() };
    /// assert!(!bad.is_valid());
    /// ```
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if self.chunk_size == 0 || self.chunk_size > 256 {
            return false;
        }
        if self.draw_distance == 0 {
            return false;
        }
        if !(16..=64).contains(&self.voronoi_site_count) {
            return false;
        }
        if !(100.0..=100_000.0).contains(&self.voronoi_span) {
            return false;
        }
        if !(0.1..=10.0).contains(&self.shepard_power) {
            return false;
        }
        if !(1e-12..=1e-3).contains(&self.shepard_epsilon) {
            return false;
        }
        // Zone weights must be non-negative and sum ~1.0.
        let sum: f64 = self.zone_weights.iter().sum();
        if (sum - 1.0).abs() > 1e-6 {
            return false;
        }
        if self.zone_weights.iter().any(|&w| !(0.0..=1.0).contains(&w)) {
            return false;
        }
        // Per-zone params.
        for z in &self.zones {
            if z.height_min > z.height_max {
                return false;
            }
            if !(0.0..=1.0).contains(&z.density) {
                return false;
            }
            if z.block_size == 0 || z.palette_count == 0 {
                return false;
            }
        }
        // Interior ranges.
        if self.interior_width_range[0] > self.interior_width_range[1]
            || self.interior_height_range[0] > self.interior_height_range[1]
        {
            return false;
        }
        if self.interior_width_range[0] == 0 || self.interior_height_range[0] == 0 {
            return false;
        }
        // Interior floor mapping and blueprints.
        if !(1e-6..=1000.0).contains(&self.interior_floor_height) {
            return false;
        }
        if self.interior_max_floors == 0 {
            return false;
        }
        for bp in &self.interior_blueprints {
            if bp.core_size == 0 {
                return false;
            }
            if usize::from(bp.room_count) > crate::layout::MAX_BLUEPRINT_ROOMS {
                return false;
            }
            for r in bp.room_slice() {
                if r.weight < 0.0
                    || r.min_w == 0
                    || r.max_w < r.min_w
                    || r.min_d == 0
                    || r.max_d < r.min_d
                {
                    return false;
                }
            }
        }
        true
    }

    /// Load a `WorldConfig` from a TOML or JSON file.
    ///
    /// The format is sniffed from the file extension (`.toml` → TOML,
    /// `.json` → JSON). If the extension is missing or unknown, TOML is tried
    /// first, then JSON.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// use urbix::config::WorldConfig;
    /// let cfg = WorldConfig::from_file("urbix.toml").unwrap();
    /// assert!(cfg.is_valid());
    /// ```
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;
        Self::from_str_with_path(&content, path)
    }

    /// Parse a `WorldConfig` from a string with a path hint for format sniffing.
    pub fn from_str_with_path(content: &str, path: &Path) -> anyhow::Result<Self> {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        match ext {
            "toml" => Self::from_toml_str(content),
            "json" => Self::from_json_str(content),
            _ => {
                // Try TOML first, then JSON.
                Self::from_toml_str(content).or_else(|_| Self::from_json_str(content))
            }
        }
    }

    /// Parse a `WorldConfig` from a TOML string.
    pub fn from_toml_str(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Parse a `WorldConfig` from a JSON string.
    pub fn from_json_str(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// Retrieve the `ZoneParams` for a given `ZoneType` from this config.
    ///
    /// Replaces the former global `zone_defaults(zone)` free function. The
    /// global remains as a deprecated shim that returns `WorldConfig::default()`
    /// values for backward compatibility.
    #[must_use]
    pub fn zone_params_for(&self, zone: ZoneType) -> ZoneParams {
        self.zones[zone as usize]
    }

    /// Retrieve the hue for a given `ZoneType`.
    #[must_use]
    pub fn hue_for(&self, zone: ZoneType) -> [u8; 3] {
        self.zone_hues[zone as usize]
    }

    /// Blend per-zone parameters from a fuzzy affinity vector using this
    /// config's `zones` (modular customization, Milestone 8).
    ///
    /// Mirrors `crate::zones::zone_params` but reads from `self.zones` instead
    /// of the global `zone_defaults`. The global function remains for backward
    /// compatibility and returns `WorldConfig::default()` values.
    #[must_use]
    pub fn blended_zone_params(&self, affinity: &[f32; crate::zones::ZONE_COUNT]) -> ZoneParams {
        let mut total = 0.0f32;
        let mut min_sum = 0.0f32;
        let mut max_sum = 0.0f32;
        let mut density_sum = 0.0f32;
        let mut block_sum = 0u32;
        let mut palette_sum = 0.0f32;
        for (i, w) in affinity.iter().enumerate() {
            total += *w;
            let d = self.zones[i];
            min_sum += *w * d.height_min;
            max_sum += *w * d.height_max;
            density_sum += *w * d.density;
            block_sum += (*w * f32::from(d.block_size)) as u32;
            palette_sum += *w * f32::from(d.palette_count);
        }
        if total <= f32::EPSILON {
            return self.zones[ZoneType::Residential as usize];
        }
        let inv = 1.0 / total;
        ZoneParams {
            height_min: min_sum * inv,
            height_max: max_sum * inv,
            density: density_sum * inv,
            block_size: (block_sum as f32 * inv).round() as u8,
            palette_count: (palette_sum * inv).round() as u8,
        }
    }

    /// Retrieve the interior layout [`Blueprint`] for a given [`ZoneType`].
    ///
    /// If a zone's blueprint is empty (no room templates configured), falls back
    /// to [`blueprint_defaults`] so the generator always has a valid rule table.
    #[must_use]
    pub fn blueprint_for(&self, zone: ZoneType) -> Blueprint {
        let bp = self.interior_blueprints[zone as usize];
        if bp.is_empty() {
            blueprint_defaults(zone)
        } else {
            bp
        }
    }

    /// Build an [`InteriorContext`] for a built cell from this config.
    ///
    /// The context is the exterior→interior bridge: it derives the floor count
    /// from the building height using this config's `interior_floor_height` /
    /// `interior_max_floors`, and records the footprint, zone, and palette so
    /// the generator reacts to them deterministically.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // thin wrapper over the flat context record
    pub fn interior_context(
        &self,
        id: InteriorId,
        zone: ZoneType,
        zone_affinity: &[f32; crate::zones::ZONE_COUNT],
        height: f32,
        footprint_w: u8,
        footprint_d: u8,
        palette_id: u8,
        seed: u64,
    ) -> InteriorContext {
        InteriorContext::new(
            id,
            zone,
            *zone_affinity,
            height,
            self.interior_floor_height,
            self.interior_max_floors,
            footprint_w,
            footprint_d,
            palette_id,
            seed,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::WorldConfig;

    #[test]
    fn default_is_valid() {
        let cfg = WorldConfig::default();
        assert_eq!(cfg.chunk_size, 32);
        assert_eq!(cfg.voronoi_site_count, 32);
        assert_eq!(cfg.voronoi_span, 10_000.0);
        assert!(cfg.is_valid());
    }

    #[test]
    fn rejects_bad_chunk_size() {
        let cfg = WorldConfig {
            chunk_size: 0,
            ..Default::default()
        };
        assert!(!cfg.is_valid());
        let cfg = WorldConfig {
            chunk_size: 512,
            ..Default::default()
        };
        assert!(!cfg.is_valid());
    }

    #[test]
    fn rejects_bad_site_count() {
        // Site counts outside the supported 16..=64 band are invalid.
        let too_low = WorldConfig {
            voronoi_site_count: 4,
            ..Default::default()
        };
        let too_high = WorldConfig {
            voronoi_site_count: 128,
            ..Default::default()
        };
        let ok = WorldConfig {
            voronoi_site_count: 48,
            ..Default::default()
        };
        assert!(!too_low.is_valid());
        assert!(!too_high.is_valid());
        assert!(ok.is_valid());
    }

    #[test]
    fn supports_other_chunk_sizes() {
        for size in [16u16, 32, 64, 128] {
            assert!(WorldConfig {
                chunk_size: size,
                ..Default::default()
            }
            .is_valid());
        }
    }

    #[test]
    fn from_toml_and_json_roundtrip() {
        let cfg = WorldConfig::default();
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed = WorldConfig::from_toml_str(&toml_str).unwrap();
        assert_eq!(cfg, parsed);
        let json_str = serde_json::to_string(&cfg).unwrap();
        let parsed = WorldConfig::from_json_str(&json_str).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn from_toml_overrides_seed() {
        let toml_str = r#"
            seed = 12345
            chunk_size = 16
            draw_distance = 4
            voronoi_site_count = 24
            voronoi_span = 5000.0
            shepard_power = 3.0
            shepard_epsilon = 1e-7
            zone_weights = [0.2, 0.2, 0.2, 0.2, 0.2]
            zone_hues = [[0,0,0],[1,1,1],[2,2,2],[3,3,3],[4,4,4]]
            interior_width_range = [4, 10]
            interior_height_range = [4, 10]
            [[zones]]
            height_min = 10.0
            height_max = 20.0
            density = 0.5
            block_size = 4
            palette_count = 4
            [[zones]]
            height_min = 10.0
            height_max = 20.0
            density = 0.5
            block_size = 4
            palette_count = 4
            [[zones]]
            height_min = 10.0
            height_max = 20.0
            density = 0.5
            block_size = 4
            palette_count = 4
            [[zones]]
            height_min = 10.0
            height_max = 20.0
            density = 0.5
            block_size = 4
            palette_count = 4
            [[zones]]
            height_min = 10.0
            height_max = 20.0
            density = 0.5
            block_size = 4
            palette_count = 4
        "#;
        let cfg = WorldConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(cfg.seed, 12345);
        assert_eq!(cfg.chunk_size, 16);
        assert!(cfg.is_valid());
    }
}
