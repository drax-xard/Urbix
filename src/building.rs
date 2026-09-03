//! # building.rs
//!
//! Building footprint and appearance generation for the Urbix engine.
//!
//! This module is the last step of per-cell construction. Given a cell that
//! is *not* a street (see `street.rs`), it assigns the building's height and
//! facade palette id, both derived deterministically from the cell's
//! coordinates and the world seed.
//!
//! ## Responsibilities
//!
//! - Derive a building's **height** via `hash` of the cell coordinates,
//!   clamped into the owning zone's `[height_min, height_max]` range so every
//!   built cell looks a little different while respecting its district.
//! - Select a **palette id** from the zone's facade palette, again from the
//!   hash, so neighbouring buildings vary but remain cohesive.
//! - Apply the zone's **density**: cells whose density roll falls short are
//!   left as empty lots (`height = 0`), so sparse districts have gaps.
//!
//! ## Determinism
//!
//! The same `(cell_x, cell_y, seed, zone)` always yields the same height and
//! palette. `cell_x`/`cell_y` are **absolute** world coordinates so a given
//! building cell is stable no matter which chunk generated it. This is what
//! keeps adjacent chunks consistent and the world reproducible.

use crate::hash::domain;
use crate::hash::{hash_coords, hash_unit};
use crate::zones::ZoneParams;

/// Assign a building's height and palette to a cell.
///
/// Returns `(height, palette_id)`. An **empty lot** — either because the zone
/// has sparse density or this cell rolled under the threshold — yields
/// `(0.0, 0)`, which the caller treats as "no building".
///
/// ## Example
///
/// ```
/// use urbix::building::assign_building;
/// use urbix::zones::{ZoneType, zone_defaults};
/// use urbix::hash::hash_unit;
///
/// let params = zone_defaults(ZoneType::Downtown);
/// let (h, _p) = assign_building(3, 3, &params, 445566);
/// // A downtown cell that is built sits within the zone's height range.
/// if h > 0.0 {
///     assert!((zone_defaults(ZoneType::Downtown).height_min..=zone_defaults(ZoneType::Downtown).height_max).contains(&h));
/// }
/// # let _ = hash_unit(0, 0, 0, 0);
/// ```
#[must_use]
pub fn assign_building(cell_x: i64, cell_y: i64, params: &ZoneParams, seed: u64) -> (f32, u8) {
    // Density gate: hash a [0,1) roll for this cell; if it exceeds the zone's
    // density the lot stays empty. Distinct domain so it doesn't correlate
    // with the height or palette draws.
    let roll = hash_unit(cell_x, cell_y, seed, domain::DENSITY);
    if roll > params.density {
        return (0.0, 0);
    }

    // Height: map a [0,1) hash into the zone's height band.
    let t = hash_unit(cell_x, cell_y, seed, domain::HEIGHT);
    let height = params.height_min + t * (params.height_max - params.height_min);
    let height = height.max(0.0); // guards against negative zone minima

    // Palette: pick an index within the zone's palette count.
    let raw = hash_coords(cell_x, cell_y, seed, domain::PALETTE);
    let palette = (raw % (params.palette_count.max(1) as u64)) as u8;

    (height, palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zones::{zone_defaults, ZoneType};

    #[test]
    fn building_is_deterministic() {
        let params = zone_defaults(ZoneType::Downtown);
        assert_eq!(
            assign_building(3, 3, &params, 445566),
            assign_building(3, 3, &params, 445566)
        );
        assert_eq!(
            assign_building(-7, 5, &params, 99),
            assign_building(-7, 5, &params, 99)
        );
    }

    #[test]
    fn built_heights_stay_in_zone_range() {
        // Downtown has density 0.95, so most cells build; check those that do.
        let params = zone_defaults(ZoneType::Downtown);
        let mut built = 0;
        for x in 1..40 {
            for y in 1..40 {
                let (h, _) = assign_building(x, y, &params, 42);
                if h > 0.0 {
                    built += 1;
                    assert!(h >= params.height_min && h <= params.height_max, "h={h}");
                } else {
                    assert_eq!(h, 0.0);
                }
            }
        }
        // A dense zone should build the vast majority of cells.
        assert!(
            built as f64 / (39.0 * 39.0) > 0.8,
            "built ratio too low: {built}"
        );
    }

    #[test]
    fn sparse_zone_builds_few_cells() {
        let params = zone_defaults(ZoneType::Park); // density 0.10
        let mut built = 0;
        for x in 1..60 {
            for y in 1..60 {
                let (h, _) = assign_building(x, y, &params, 7);
                if h > 0.0 {
                    built += 1;
                }
            }
        }
        // A park should be mostly empty lots.
        assert!(
            built as f64 / (59.0 * 59.0) < 0.2,
            "park built too much: {built}"
        );
    }

    #[test]
    fn palette_is_bounded_by_zone_count() {
        let params = zone_defaults(ZoneType::Commercial);
        let n = params.palette_count as u64;
        for x in 0..50 {
            for y in 0..50 {
                let (_, p) = assign_building(x, y, &params, 123);
                assert!((p as u64) < n, "palette {p} out of range");
            }
        }
    }

    #[test]
    fn zero_size_ranges_do_not_panic() {
        let params = ZoneParams {
            height_min: 5.0,
            height_max: 5.0,
            density: 1.0,
            block_size: 1,
            palette_count: 0,
        };
        // A zero-width height band and zero palette count are degenerate but
        // must not panic or produce NaN.
        let (h, p) = assign_building(1, 1, &params, 1);
        assert!(!h.is_nan());
        assert_eq!(p, 0);
    }
}
