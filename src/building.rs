//! # building.rs
//!
//! Building footprint and appearance generation for the Urbix engine.
//!
//! Milestone 11 makes buildings coherent at **lot** scale: one height,
//! palette, and density fate per lot (`lot::LotSlot`), with only a small
//! per-cell jitter. A block frontage therefore reads as a continuous street
//! wall instead of sawtooth noise, while every building still varies.
//!
//! ## Responsibilities
//!
//! - Derive a lot's **fate** (build vs empty) from the lot hash plus a
//!   per-block noise draw, so vacancy clumps into yards and gardens instead
//!   of speckling cells.
//! - Derive a lot's **height** from the lot hash mapped into the zone band,
//!   scaled by the block noise and the CBD peak factor, with a corner-lot
//!   bonus — then jitter ±10% per cell for life.
//! - Clamp the height to the lot's footprint (`slenderness_max` × narrowest
//!   side in metres) so towers read as broad masses, never spikes.
//! - Select one **palette id** per lot from the zone's facade count.
//!
//! ## Determinism
//!
//! The same `(lot_id, cell, seed, zone)` always yields the same height and
//! palette. `lot_id` is stable per `(block, slot)` (`lot.rs`), so a given
//! lot is identical no matter which chunk generated it.

use crate::hash::domain;
use crate::hash::{hash_coords, hash_unit};
use crate::massing::{massing_for, MassingParams};
use crate::zones::ZoneParams;

/// Assign a building's height and palette to a cell within a lot.
///
/// Returns `(height, palette_id)`. An **empty lot** yields `(0.0, 0)` for
/// every cell in it. Inputs:
///
/// - `lot_id`: stable per-lot key from `lot::lot_slot` — every cell in the
///   lot passes the same id, which is what makes the lot uniform.
/// - `corner`: lot touches streets on two axes; earns a ~20% height bonus.
/// - `block_clump`: per-block `lot::block_noise` draw; lots whose combined
///   roll falls short stay empty together (clumped vacancy).
/// - `cbd_boost`: `region::VoronoiDiagram::cbd_factor` at the cell; peaks the
///   downtown core and tapers outward.
/// - `height_boost`: combined landmark / special-block pre-step multiplier
///   (`1.0` = none). Applied to `H` *before* massing steps so boosted towers
///   keep their podium + crown, just taller (`chunk.rs` computes it; with
///   massing off it stays `1.0` and the legacy post boosts run instead).
/// - `cell_jitter`: pass the cell's world coords for the ±10% per-cell life;
///   pass `None` in tests to assert exact lot uniformity.
/// - `lot_footprint`: the lot's `(width, depth)` in cells from
///   `lot::lot_rect` — bounds the slenderness clamp (see
///   [`slenderness_cap`]) and sizes the massing steps.
/// - `lot_offset`: the cell's `(ox, oz)` position inside its lot from
///   `lot::lot_offset` — drives the massing ring/roof math. Ignored when
///   `massing` is `None`.
/// - `massing`: split-grammar rules for the dominant zone, or `None` to keep
///   the legacy flat extrude (M17 `massing_mode = 0`). `Some` with a flat
///   table is bit-identical to `None` (see `massing::is_flat`).
/// - `floor_height`: metres per storey (`WorldConfig::interior_floor_height`);
///   only read when `massing` is `Some`.
///
/// ## Example
///
/// ```
/// use urbix::building::assign_building;
/// use urbix::zones::{ZoneType, zone_defaults};
///
/// let params = zone_defaults(ZoneType::Downtown);
/// let (h, _p) = assign_building(0x9e37_79b9_7f4a_7c15, true, 0.9, 1.2, 1.0, Some((3, 3)), (5, 5), (2, 2), None, 4.0, &params, 445566);
/// if h > 0.0 {
///     assert!((params.height_min..=params.height_max * 1.5).contains(&h));
/// }
/// ```
#[must_use]
#[allow(clippy::too_many_arguments)] // one bundle per hierarchy level; splitting hides the pipeline
pub fn assign_building(
    lot_id: u64,
    corner: bool,
    block_clump: f32,
    cbd_boost: f32,
    height_boost: f32,
    cell_jitter: Option<(i64, i64)>,
    lot_footprint: (u8, u8),
    lot_offset: (u8, u8),
    massing: Option<&MassingParams>,
    floor_height: f32,
    params: &ZoneParams,
    seed: u64,
) -> (f32, u8) {
    // Lot fate: combine the lot draw with the block clump so vacancy clusters
    // per block instead of speckling. Distinct domains keep fate independent
    // of height/palette.
    let lot_roll = hash_unit(lot_id as i64, (lot_id >> 32) as i64, seed, domain::DENSITY);
    if lot_roll * 0.65 + block_clump * 0.35 > params.density {
        return (0.0, 0);
    }

    // Lot height: map the lot draw into the zone band, then shape it.
    let t = hash_unit(
        (lot_id >> 32) as i64,
        lot_id as i64,
        seed,
        domain::LOT_HEIGHT,
    );
    // Right-skew the draw (`t²`) so most lots sit low in the band and tall
    // towers stay rare — a uniform draw puts the median at the band midpoint,
    // which reads as a district of high-rises. The low end is untouched
    // (a `t` of 0 still maps to `height_min`), the top stays reachable but
    // needs a `t` near 1.
    let t = t * t;
    let mut height = params.height_min + t * (params.height_max - params.height_min);
    // Block-scale variation: ±15% so neighbouring lots differ in rhythm.
    height *= 0.85 + block_clump * 0.30;
    // Downtown peak: boost toward the CBD, taper outward.
    height *= cbd_boost;
    // Corner lots anchor the intersection: +20%.
    if corner {
        height *= 1.2;
    }
    // Landmark / special-block pre-step boost (Milestone 17): boosted towers
    // step from a taller H, so the podium ring and crown survive the boost.
    // `1.0` is the exact identity (IEEE), keeping the legacy path untouched.
    height *= height_boost;
    // Split-grammar massing (Milestone 17): podium/roof/crown steps per cell.
    // Skipped entirely for `None` (legacy mode); flat tables are the identity.
    if let Some(mp) = massing {
        height = massing_for(
            lot_id,
            lot_footprint,
            lot_offset,
            height,
            mp,
            seed,
            floor_height,
        );
    }
    // Per-cell life: ±10% deterministic jitter around the lot height.
    if let Some((cx, cy)) = cell_jitter {
        let j = hash_unit(cx, cy, seed, domain::HEIGHT);
        height *= 0.9 + j * 0.2;
    }
    // Clamp into a sane envelope: never below 0, never absurdly above the
    // band (bonuses stack at most ~1.3 × 1.2 × 1.1 ≈ 1.7×; clamp at 1.3× max
    // to keep outliers believable while preserving landmark variance).
    // Then the slenderness cap: no lot outgrows its own footprint, so the
    // skyline reads as broad masses instead of spikes.
    let height = height
        .max(0.0)
        .min(params.height_max * 1.3 + 1.0)
        .min(slenderness_cap(params, lot_footprint.0, lot_footprint.1));

    // Palette: one per lot.
    let raw = hash_coords(lot_id as i64, (lot_id >> 32) as i64, seed, domain::PALETTE);
    let palette = (raw % (params.palette_count.max(1) as u64)) as u8;

    (height, palette)
}

/// Slenderness cap for a lot: the tallest believable building on a
/// `lot_w × lot_d` (cells) footprint under these zone params.
///
/// Returns `slenderness_max × min(lot_w, lot_d) × 4.0` metres (the M11 scale
/// canon: 1 cell = 4 m), or `f32::INFINITY` when the policy is disabled
/// (`slenderness_max == 0`). Pure geometry — no hash — so `chunk.rs` reuses
/// it after the landmark/special-block boosts and both sites always agree.
///
/// ## Example
///
/// ```
/// use urbix::building::slenderness_cap;
/// use urbix::zones::{ZoneType, zone_defaults};
///
/// let params = zone_defaults(ZoneType::Downtown); // slenderness_max 4
/// assert_eq!(slenderness_cap(&params, 5, 5), 80.0);
/// ```
#[must_use]
pub fn slenderness_cap(params: &ZoneParams, lot_w: u8, lot_d: u8) -> f32 {
    if params.slenderness_max == 0 {
        return f32::INFINITY;
    }
    f32::from(params.slenderness_max) * f32::from(lot_w.min(lot_d).max(1)) * 4.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lot::{block_loc, lot_slot, DistrictFrame};
    use crate::zones::{zone_defaults, ZoneType};

    fn lot_of(x: i64, y: i64, seed: u64) -> (u64, bool) {
        let frame = DistrictFrame::identity();
        let params = zone_defaults(ZoneType::Downtown);
        let loc = block_loc(x, y, params.block_size, &frame);
        let slot = lot_slot(&loc, params.block_size, seed);
        (slot.lot_id, slot.corner)
    }

    #[test]
    fn building_is_deterministic() {
        let params = zone_defaults(ZoneType::Downtown);
        let (lot, corner) = lot_of(3, 3, 445566);
        let a = assign_building(
            lot,
            corner,
            0.9,
            1.0,
            1.0,
            Some((3, 3)),
            (5, 5),
            (1, 1),
            None,
            4.0,
            &params,
            445566,
        );
        let b = assign_building(
            lot,
            corner,
            0.9,
            1.0,
            1.0,
            Some((3, 3)),
            (5, 5),
            (1, 1),
            None,
            4.0,
            &params,
            445566,
        );
        assert_eq!(a, b);
        let (lot2, corner2) = lot_of(-7, 5, 99);
        assert_eq!(
            assign_building(
                lot2,
                corner2,
                0.5,
                1.0,
                1.0,
                Some((-7, 5)),
                (5, 5),
                (0, 0),
                None,
                4.0,
                &params,
                99
            ),
            assign_building(
                lot2,
                corner2,
                0.5,
                1.0,
                1.0,
                Some((-7, 5)),
                (5, 5),
                (0, 0),
                None,
                4.0,
                &params,
                99
            )
        );
    }

    #[test]
    fn lot_cells_share_palette_and_near_height() {
        // Two cells of one lot share the palette; heights stay within the
        // ±10% jitter band of each other. (The split axis is hashed per
        // block, so scan for a same-lot pair instead of assuming one.)
        let params = zone_defaults(ZoneType::Commercial);
        let frame = DistrictFrame::identity();
        let mut pair = None;
        for x in 1..20 {
            let a = block_loc(x, 5, params.block_size, &frame);
            let b = block_loc(x + 1, 5, params.block_size, &frame);
            let (sa, sb) = (
                lot_slot(&a, params.block_size, 42),
                lot_slot(&b, params.block_size, 42),
            );
            if sa.lot_id == sb.lot_id {
                pair = Some(((x, 5, sa), (x + 1, 5, sb)));
                break;
            }
        }
        // Fall back to the z axis if the block splits along z.
        if pair.is_none() {
            for z in 1..20 {
                let a = block_loc(5, z, params.block_size, &frame);
                let b = block_loc(5, z + 1, params.block_size, &frame);
                let (sa, sb) = (
                    lot_slot(&a, params.block_size, 42),
                    lot_slot(&b, params.block_size, 42),
                );
                if sa.lot_id == sb.lot_id {
                    pair = Some(((5, z, sa), (5, z + 1, sb)));
                    break;
                }
            }
        }
        let ((ax, ay, sa), (bx, by, sb)) = pair.expect("no same-lot pair found");
        let off_a = crate::lot::lot_offset(
            &block_loc(ax, ay, params.block_size, &frame),
            params.block_size,
            42,
        );
        let off_b = crate::lot::lot_offset(
            &block_loc(bx, by, params.block_size, &frame),
            params.block_size,
            42,
        );
        let (ha, pa) = assign_building(
            sa.lot_id,
            sa.corner,
            0.9,
            1.0,
            1.0,
            Some((ax, ay)),
            (5, 5),
            off_a,
            None,
            4.0,
            &params,
            42,
        );
        let (hb, pb) = assign_building(
            sb.lot_id,
            sb.corner,
            0.9,
            1.0,
            1.0,
            Some((bx, by)),
            (5, 5),
            off_b,
            None,
            4.0,
            &params,
            42,
        );
        assert_eq!(pa, pb);
        if ha > 0.0 && hb > 0.0 {
            let ratio = ha / hb;
            assert!(
                (0.8..=1.25).contains(&ratio),
                "jitter band broken: {ha}/{hb}"
            );
        }
    }

    #[test]
    fn built_heights_stay_in_zone_envelope() {
        // Downtown is dense; built lots stay within the shaped envelope.
        let params = zone_defaults(ZoneType::Downtown);
        let frame = DistrictFrame::identity();
        let mut built = 0;
        for x in 1..40 {
            for y in 1..40 {
                let loc = block_loc(x, y, params.block_size, &frame);
                let slot = lot_slot(&loc, params.block_size, 42);
                let clump = crate::lot::block_noise(loc.bx, loc.bz, 42);
                let rect = crate::lot::lot_rect(&loc, params.block_size, 42);
                let off = crate::lot::lot_offset(&loc, params.block_size, 42);
                let (h, _) = assign_building(
                    slot.lot_id,
                    slot.corner,
                    clump,
                    1.0,
                    1.0,
                    Some((x, y)),
                    rect,
                    off,
                    None,
                    4.0,
                    &params,
                    42,
                );
                if h > 0.0 {
                    built += 1;
                    assert!(h <= params.height_max * 1.3 + 1.0, "h={h} escapes envelope");
                } else {
                    assert_eq!(h, 0.0);
                }
            }
        }
        assert!(
            built as f64 / (39.0 * 39.0) > 0.5,
            "built ratio too low: {built}"
        );
    }

    #[test]
    fn sparse_zone_builds_few_lots() {
        let params = zone_defaults(ZoneType::Park); // density 0.10
        let frame = DistrictFrame::identity();
        let mut built = 0;
        let mut total = 0;
        for x in 1..60 {
            for y in 1..60 {
                total += 1;
                let loc = block_loc(x, y, params.block_size, &frame);
                let slot = lot_slot(&loc, params.block_size, 7);
                let clump = crate::lot::block_noise(loc.bx, loc.bz, 7);
                let rect = crate::lot::lot_rect(&loc, params.block_size, 7);
                let off = crate::lot::lot_offset(&loc, params.block_size, 7);
                let (h, _) = assign_building(
                    slot.lot_id,
                    slot.corner,
                    clump,
                    1.0,
                    1.0,
                    Some((x, y)),
                    rect,
                    off,
                    None,
                    4.0,
                    &params,
                    7,
                );
                if h > 0.0 {
                    built += 1;
                }
            }
        }
        assert!(
            (built as f64) / (total as f64) < 0.25,
            "park built too much: {built}/{total}"
        );
    }

    #[test]
    fn palette_is_bounded_by_zone_count() {
        let params = zone_defaults(ZoneType::Commercial);
        let n = params.palette_count as u64;
        for x in 0..50 {
            for y in 0..50 {
                let (lot, corner) = lot_of(x, y, 123);
                let (_, p) = assign_building(
                    lot,
                    corner,
                    0.9,
                    1.0,
                    1.0,
                    Some((x, y)),
                    (5, 5),
                    (0, 0),
                    None,
                    4.0,
                    &params,
                    123,
                );
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
            arterial_every: 0,
            slenderness_max: 0,
        };
        let (h, p) = assign_building(
            1,
            false,
            1.0,
            1.0,
            1.0,
            Some((1, 1)),
            (4, 4),
            (0, 0),
            None,
            4.0,
            &params,
            1,
        );
        assert!(!h.is_nan());
        assert_eq!(p, 0);
    }

    #[test]
    fn slenderness_cap_caps_narrow_lots() {
        // Downtown band (24-110 m) on a 2-cell-wide lot: 4 × 2 × 4 = 32 m lid.
        let params = zone_defaults(ZoneType::Downtown);
        assert_eq!(slenderness_cap(&params, 2, 6), 32.0);
        assert_eq!(slenderness_cap(&params, 5, 5), 80.0);
        // Disabled policy never caps.
        let off = ZoneParams {
            slenderness_max: 0,
            ..zone_defaults(ZoneType::Downtown)
        };
        assert_eq!(slenderness_cap(&off, 1, 1), f32::INFINITY);
    }

    #[test]
    fn narrow_lot_heights_respect_slenderness() {
        // Sweep downtown lots: every built height stays under its own
        // footprint cap, even with maxed bonuses — including the M17
        // landmark/tower-park pre-step boost (1.5 × 1.35), which the steps
        // must absorb before the clamp.
        use crate::massing::massing_defaults;
        let params = zone_defaults(ZoneType::Downtown);
        let frame = DistrictFrame::identity();
        let mut capped = 0;
        let mut built = 0;
        for x in 1..60 {
            for y in 1..60 {
                let loc = block_loc(x, y, params.block_size, &frame);
                let slot = lot_slot(&loc, params.block_size, 42);
                let rect = crate::lot::lot_rect(&loc, params.block_size, 42);
                let off = crate::lot::lot_offset(&loc, params.block_size, 42);
                for boost in [1.0, 1.5 * 1.35] {
                    let (h, _) = assign_building(
                        slot.lot_id,
                        true, // corner bonus on, worst case
                        1.0,  // max block clump
                        1.5,  // max CBD boost
                        boost,
                        Some((x, y)),
                        rect,
                        off,
                        Some(&massing_defaults(ZoneType::Downtown)),
                        4.0,
                        &params,
                        42,
                    );
                    if h > 0.0 {
                        built += 1;
                        let cap = slenderness_cap(&params, rect.0, rect.1);
                        assert!(h <= cap + 1e-3, "h={h} over cap={cap} rect={rect:?}");
                        if h >= cap - 1e-3 {
                            capped += 1;
                        }
                    }
                }
            }
        }
        assert!(built > 0, "no built lots sampled");
        assert!(capped > 0, "clamp never engaged — is it wired?");
    }

    #[test]
    fn none_and_flat_some_agree_exactly() {
        // Plumbing transparency: `None` (legacy mode) and `Some` with a flat
        // table take the same code path — the gateway the chunk-level
        // `massing_mode = 0` byte-equality test rests on.
        use crate::massing::MassingParams;
        let flat = MassingParams {
            podium_floors: 0,
            tower_inset: 0,
            setback_every: 0,
            roof_mode: 0,
            crown_share: 0.0,
        };
        let params = zone_defaults(ZoneType::Downtown);
        let frame = DistrictFrame::identity();
        for x in 1..40 {
            for y in 1..40 {
                let loc = block_loc(x, y, params.block_size, &frame);
                let slot = lot_slot(&loc, params.block_size, 42);
                let clump = crate::lot::block_noise(loc.bx, loc.bz, 42);
                let rect = crate::lot::lot_rect(&loc, params.block_size, 42);
                let off = crate::lot::lot_offset(&loc, params.block_size, 42);
                let a = assign_building(
                    slot.lot_id,
                    slot.corner,
                    clump,
                    1.0,
                    1.0,
                    Some((x, y)),
                    rect,
                    off,
                    None,
                    4.0,
                    &params,
                    42,
                );
                let b = assign_building(
                    slot.lot_id,
                    slot.corner,
                    clump,
                    1.0,
                    1.0,
                    Some((x, y)),
                    rect,
                    off,
                    Some(&flat),
                    4.0,
                    &params,
                    42,
                );
                assert_eq!(a, b, "legacy path diverges at ({x}, {y})");
            }
        }
    }

    #[test]
    fn legacy_path_ignores_offset_and_floor_height() {
        // With `None` the new inputs are dead: varying them changes nothing,
        // so stale callers cannot perturb legacy output.
        let params = zone_defaults(ZoneType::Commercial);
        let (lot, corner) = lot_of(11, 11, 7);
        let a = assign_building(
            lot,
            corner,
            0.9,
            1.0,
            1.0,
            Some((11, 11)),
            (6, 6),
            (0, 0),
            None,
            4.0,
            &params,
            7,
        );
        let b = assign_building(
            lot,
            corner,
            0.9,
            1.0,
            1.0,
            Some((11, 11)),
            (6, 6),
            (5, 5),
            None,
            99.0,
            &params,
            7,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn pre_step_boost_keeps_podium() {
        // Boosts multiply H *before* steps: the edge ring still caps at the
        // podium while the core grows with the boost (spec: boosted towers
        // keep podium + crown, just taller). Fixed-band zone params make H
        // exact (46 m: clears the 12 m podium, clears every clamp).
        use crate::massing::{massing_defaults, MassingParams};
        let no_crown = MassingParams {
            crown_share: 0.0,
            ..massing_defaults(ZoneType::Downtown)
        };
        let params = ZoneParams {
            height_min: 40.0,
            height_max: 40.0,
            density: 1.0,
            block_size: 11,
            palette_count: 6,
            arterial_every: 4,
            slenderness_max: 5,
        };
        let lot = 12345u64;
        let edge_plain = assign_building(
            lot,
            false,
            0.0,
            1.0,
            1.0,
            None,
            (7, 7),
            (0, 3),
            Some(&no_crown),
            4.0,
            &params,
            69420,
        );
        let core_plain = assign_building(
            lot,
            false,
            0.0,
            1.0,
            1.0,
            None,
            (7, 7),
            (3, 3),
            Some(&no_crown),
            4.0,
            &params,
            69420,
        );
        let edge_boost = assign_building(
            lot,
            false,
            0.0,
            1.0,
            1.5,
            None,
            (7, 7),
            (0, 3),
            Some(&no_crown),
            4.0,
            &params,
            69420,
        );
        let core_boost = assign_building(
            lot,
            false,
            0.0,
            1.0,
            1.5,
            None,
            (7, 7),
            (3, 3),
            Some(&no_crown),
            4.0,
            &params,
            69420,
        );
        // H = 40 × 0.85 (min clump) = 34: clears the 12 m podium and every
        // clamp (1.6× cap is 65, slenderness 140), so the boost ratio reads
        // exactly. (`min` returns the cap operand bit-exactly; the core
        // needs an epsilon since 0.85 is not exact binary.)
        assert_eq!(edge_plain.0, 12.0);
        assert_eq!(edge_boost.0, 12.0);
        assert!((core_plain.0 - 34.0).abs() < 1e-3);
        assert!((core_boost.0 / core_plain.0 - 1.5).abs() < 1e-5);
    }
}
