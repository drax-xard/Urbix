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

use serde::{Deserialize, Serialize};

/// The number of distinct zone types.
pub const ZONE_COUNT: usize = 5;

/// The five urban region types the engine can generate.
///
/// The order of variants is fixed and matches the affinity-vector layout in
/// [`ZoneParams`] and [`zone_params`]: index `i` corresponds to variant `i`,
/// so `zone_affinity[ZoneType::Downtown as usize]` is Downtown's weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ZoneType {
    /// Dense, tall skyscrapers.
    Downtown = 0,
    /// Tranquil, low-rise, tree-lined.
    Residential = 1,
    /// Busy mid-rise with bright/neon palettes.
    Commercial = 2,
    /// Grimy, wide low warehouses, open lots.
    Industrial = 3,
    /// Low or zero buildings, foliage, open ground.
    Park = 4,
}

impl ZoneType {
    /// All five zone variants, in variant order.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::zones::{ZoneType, ZONE_COUNT};
    ///
    /// assert_eq!(ZoneType::all().len(), ZONE_COUNT);
    /// assert_eq!(ZoneType::all()[0], ZoneType::Downtown);
    /// ```
    #[must_use]
    pub const fn all() -> [ZoneType; ZONE_COUNT] {
        [
            ZoneType::Downtown,
            ZoneType::Residential,
            ZoneType::Commercial,
            ZoneType::Industrial,
            ZoneType::Park,
        ]
    }
}

/// Per-zone parameters that shape building density, height, and block layout.
///
/// This is a plain `#[repr(C)]` struct of scalar data so it can feed the
/// FFI layer and be interpolated across a fuzzy [zone-affinity] vector.
///
/// [zone-affinity]: crate::zones::zone_params
///
/// ## Scale canon (Milestone 11)
///
/// 1 cell = 4 m. `block_size` is in cells; multiply by 4 for metres.
/// `arterial_every` counts ordinary streets: every K-th street is a 2-cell
/// arterial avenue (`K == 0` means no arterials).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct ZoneParams {
    /// Minimum building height (world units) for the zone.
    pub height_min: f32,
    /// Maximum building height (world units) for the zone.
    pub height_max: f32,
    /// Probability in [0, 1] that a non-street cell gets a building buildable at all.
    pub density: f32,
    /// Edge length (in cells) of a single street block for this zone.
    pub block_size: u8,
    /// Number of distinct facade palettes available to this zone.
    pub palette_count: u8,
    /// Every K-th street is a 2-cell arterial avenue. `0` disables arterials.
    /// Snapped (not blended) from the dominant zone; see [`zone_params`].
    #[serde(default = "default_arterial_every")]
    pub arterial_every: u8,
}

/// Serde default for [`ZoneParams::arterial_every`] so pre-M11 config files
/// without the field keep parsing (old files get per-zone defaults below
/// only when the whole `zones` array is omitted; files that specify `zones`
/// explicitly without the field get `4`, the downtown/commercial norm).
#[must_use]
pub const fn default_arterial_every() -> u8 {
    4
}

/// Default parameters for a single [`ZoneType`].
///
/// These are the canonical per-zone profiles used as the blend end-points in
/// [`zone_params`]. Heights deliberately overlap between zones so fuzzy
/// borders produce gradual, plausible transitions.
///
/// Block sizes follow the Milestone 11 scale canon (1 cell = 4 m):
/// downtown 10 (40 m, permeable), residential 10, commercial 9,
/// industrial 14 (56 m yard edge), park 18 (green mass).
#[must_use]
pub fn zone_defaults(zone: ZoneType) -> ZoneParams {
    match zone {
        ZoneType::Downtown => ZoneParams {
            height_min: 40.0,
            height_max: 200.0,
            density: 0.95,
            block_size: 11,
            palette_count: 6,
            arterial_every: 4,
        },
        ZoneType::Residential => ZoneParams {
            height_min: 4.0,
            height_max: 18.0,
            density: 0.80,
            block_size: 10,
            palette_count: 5,
            arterial_every: 5,
        },
        ZoneType::Commercial => ZoneParams {
            height_min: 12.0,
            height_max: 60.0,
            density: 0.90,
            block_size: 9,
            palette_count: 7,
            arterial_every: 4,
        },
        ZoneType::Industrial => ZoneParams {
            height_min: 6.0,
            height_max: 25.0,
            density: 0.70,
            block_size: 14,
            palette_count: 4,
            arterial_every: 6,
        },
        ZoneType::Park => ZoneParams {
            height_min: 0.0,
            height_max: 2.0,
            density: 0.10,
            block_size: 18,
            palette_count: 3,
            arterial_every: 0,
        },
    }
}

/// Blend per-zone parameters from a fuzzy zone-affinity vector.
///
/// `affinity` is a length-`ZONE_COUNT` weight vector (produced by the Voronoi
/// layer, one entry per [`ZoneType`]) expected to be non-negative. Heights,
/// density, and palette count are affinity-weighted averages. `block_size`
/// and `arterial_every` are **snapped from the dominant zone** (argmax, ties
/// toward the lower index) instead of averaged: averaging grid periods
/// produces hybrid spacings (e.g. 7/9/10) that belong to neither district,
/// while snapping keeps each block buildable and each fabric legible.
/// If the vector is all-in on a single zone the result equals that
/// zone's defaults exactly — the boundary case the tests pin down.
///
/// ## Example
///
/// ```
/// use urbix::zones::{zone_params, ZoneType, zone_defaults};
///
/// let mut downtown_only = [0.0f32; 5];
/// downtown_only[ZoneType::Downtown as usize] = 1.0;
/// assert_eq!(zone_params(&downtown_only), zone_defaults(ZoneType::Downtown));
/// ```
#[must_use]
pub fn zone_params(affinity: &[f32; ZONE_COUNT]) -> ZoneParams {
    let zones = ZoneType::all();
    let mut total = 0.0f32;
    let mut min_sum = 0.0f32;
    let mut max_sum = 0.0f32;
    let mut density_sum = 0.0f32;
    let mut palette_sum = 0.0f32;
    // Dominant zone (argmax, ties toward lower index) owns the grid period.
    let mut best_idx = 0usize;
    let mut best_w = f32::NEG_INFINITY;

    for (i, zone) in zones.iter().enumerate() {
        let w = affinity[i];
        total += w;
        let d = zone_defaults(*zone);
        min_sum += w * d.height_min;
        max_sum += w * d.height_max;
        // Density is accumulated as a weighted mean over the *same* total.
        density_sum += w * d.density;
        palette_sum += w * f32::from(d.palette_count);
        if w > best_w {
            best_w = w;
            best_idx = i;
        }
    }

    if total <= f32::EPSILON {
        // A fully-zero affinity is ambiguous; fall back to residential so the
        // caller is never handed NaNs.
        return zone_defaults(ZoneType::Residential);
    }

    let inv = 1.0 / total;
    let grid = zone_defaults(zones[best_idx]);
    ZoneParams {
        height_min: min_sum * inv,
        height_max: max_sum * inv,
        density: density_sum * inv,
        block_size: grid.block_size,
        palette_count: (palette_sum * inv).round() as u8,
        arterial_every: grid.arterial_every,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_matches_count() {
        assert_eq!(ZoneType::all().len(), ZONE_COUNT);
    }

    #[test]
    fn zone_defaults_are_sane() {
        for zone in ZoneType::all() {
            let p = zone_defaults(zone);
            assert!(p.height_min <= p.height_max, "min > max for {zone:?}");
            assert!((0.0..=1.0).contains(&p.density));
            assert!(p.block_size > 0);
            assert!(p.palette_count > 0);
            assert!(
                p.arterial_every <= 16,
                "arterial_every out of range for {zone:?}"
            );
        }
    }

    #[test]
    fn single_zone_matches_defaults_exactly() {
        for zone in ZoneType::all() {
            let mut affinity = [0.0f32; ZONE_COUNT];
            affinity[zone as usize] = 1.0;
            assert_eq!(zone_params(&affinity), zone_defaults(zone), "zone {zone:?}");
        }
    }

    #[test]
    fn zero_affinity_falls_back() {
        let z = zone_params(&[0.0f32; ZONE_COUNT]);
        assert_eq!(z, zone_defaults(ZoneType::Residential));
    }

    #[test]
    fn blend_stays_bounded() {
        // A two-zone 50/50 blend must lie between the two zones' bounds.
        let mut a = [0.0f32; ZONE_COUNT];
        a[ZoneType::Downtown as usize] = 0.5;
        a[ZoneType::Park as usize] = 0.5;
        let z = zone_params(&a);
        let d = zone_defaults(ZoneType::Downtown);
        let p = zone_defaults(ZoneType::Park);
        assert!(z.height_min >= p.height_min && z.height_max <= d.height_max);
        assert!(z.density <= d.density && z.density >= p.density);
    }

    #[test]
    fn grid_period_snaps_to_dominant_zone() {
        // Blend must not average grid periods: 60/40 downtown/park keeps the
        // downtown block, and the mirror keeps the park block.
        let mut a = [0.0f32; ZONE_COUNT];
        a[ZoneType::Downtown as usize] = 0.6;
        a[ZoneType::Park as usize] = 0.4;
        let z = zone_params(&a);
        assert_eq!(z.block_size, zone_defaults(ZoneType::Downtown).block_size);
        assert_eq!(
            z.arterial_every,
            zone_defaults(ZoneType::Downtown).arterial_every
        );
        let mut b = [0.0f32; ZONE_COUNT];
        b[ZoneType::Downtown as usize] = 0.4;
        b[ZoneType::Park as usize] = 0.6;
        let z = zone_params(&b);
        assert_eq!(z.block_size, zone_defaults(ZoneType::Park).block_size);
        // Heights still blend even though the grid snaps.
        assert!(z.height_min > zone_defaults(ZoneType::Park).height_min);
    }
}
