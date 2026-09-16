//! # street.rs
//!
//! Street grid and block subdivision for the Urbix engine.
//!
//! Milestone 11 replaces the single global graph-paper grid with a
//! **varied fabric**: each district owns an orientation frame
//! (`lot::DistrictFrame` — quantized rotation plus a gentle sine warp), and
//! streets form a two-tier hierarchy — 1-cell local streets plus 2-cell
//! arterial avenues every `ZoneParams::arterial_every` streets.
//!
//! ## Responsibilities
//!
//! - Given a cell's zone parameters, district frame, and seed, decide whether
//!   it is a street (and whether that street is an arterial).
//! - Streets carry `height = 0` and are excluded from building placement.
//! - Sidewalk rings, plazas, and block interiors are composed in `chunk.rs`
//!   on top of this query; this module answers only the street question so it
//!   stays a cheap pure function callers can reuse for neighbour checks.
//!
//! ## Determinism
//!
//! The layout is computed from **absolute** world cell coordinates in the
//! district frame, so the same cell is a street (or not) regardless of which
//! chunk asks. Frames are piecewise constant per district (`region.rs`), so
//! neighbouring districts meet with intentional jogs — the varied-fabric
//! effect — while cells inside one district share one seamless grid.

use crate::data::CellFlags;
use crate::lot::{block_loc, DistrictFrame};
use crate::zones::ZoneParams;

/// Decide whether a world cell lands on the district street grid.
///
/// Transforms `(cell_x, cell_y)` into the district frame, quantizes to the
/// block lattice, and tests the block-boundary rule: offset `0` on either
/// axis is a street. Arterial avenues — block boundaries whose block index is
/// a multiple of `arterial_every` (≥ 2) — are 2 cells wide (offsets `0` and
/// `1`) and carry `IS_ARTERIAL` in addition to `IS_STREET` so old consumers
/// still render them as roads. `arterial_every < 2` disables arterials.
/// A block size of 0 is treated as 1 to avoid modulo-by-zero.
///
/// ## Example
///
/// ```
/// use urbix::street::layout_block;
/// use urbix::data::CellFlags;
/// use urbix::lot::DistrictFrame;
/// use urbix::zones::{ZoneParams, ZoneType, zone_defaults};
///
/// let params = ZoneParams { arterial_every: 0, ..zone_defaults(ZoneType::Downtown) };
/// let frame = DistrictFrame::identity();
/// assert!(layout_block(0, 0, &params, &frame).contains(CellFlags::IS_STREET));
/// assert!(layout_block(11, 5, &params, &frame).contains(CellFlags::IS_STREET));
/// assert!(!layout_block(1, 1, &params, &frame).contains(CellFlags::IS_STREET));
/// ```
#[must_use]
pub fn layout_block(
    cell_x: i64,
    cell_y: i64,
    params: &ZoneParams,
    frame: &DistrictFrame,
) -> CellFlags {
    let loc = block_loc(cell_x, cell_y, params.block_size, frame);
    let k = i64::from(params.arterial_every);
    if k >= 2 {
        // Arterial avenues: every K-th block boundary is 2 cells wide, so both
        // the boundary row (offset 0) and its neighbour (offset 1) are paved
        // and flagged arterial. Checked before the local rule so wide rows
        // are streets even though offset 1 is off the boundary.
        let avenue_v = loc.bx.rem_euclid(k) == 0;
        let avenue_h = loc.bz.rem_euclid(k) == 0;
        if (avenue_v && (loc.rx == 0 || loc.rx == 1)) || (avenue_h && (loc.rz == 0 || loc.rz == 1))
        {
            return CellFlags::IS_STREET.insert(CellFlags::IS_ARTERIAL);
        }
    }
    if loc.rx == 0 || loc.rz == 0 {
        CellFlags::IS_STREET
    } else {
        CellFlags::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zones::{zone_defaults, ZoneType};

    fn downtown_no_arterial() -> ZoneParams {
        ZoneParams {
            arterial_every: 0,
            ..zone_defaults(ZoneType::Downtown)
        }
    }

    #[test]
    fn street_falls_on_block_boundaries() {
        // Downtown block_size 11: boundary rows/cols are streets.
        let params = downtown_no_arterial();
        let frame = DistrictFrame::identity();
        assert!(layout_block(0, 0, &params, &frame).contains(CellFlags::IS_STREET));
        assert!(layout_block(11, 0, &params, &frame).contains(CellFlags::IS_STREET));
        assert!(layout_block(0, 22, &params, &frame).contains(CellFlags::IS_STREET));
        assert!(layout_block(22, 3, &params, &frame).contains(CellFlags::IS_STREET));
        // Interior cells are not streets.
        assert_eq!(layout_block(1, 1, &params, &frame), CellFlags::NONE);
        assert_eq!(layout_block(10, 10, &params, &frame), CellFlags::NONE);
    }

    #[test]
    fn street_handles_negative_coordinates() {
        // rem_euclid keeps boundaries consistent across the sign change.
        let params = ZoneParams {
            arterial_every: 0,
            ..zone_defaults(ZoneType::Commercial)
        };
        let frame = DistrictFrame::identity();
        let b = i64::from(params.block_size);
        assert!(layout_block(-b, 2, &params, &frame).contains(CellFlags::IS_STREET));
        assert!(layout_block(b, 2, &params, &frame).contains(CellFlags::IS_STREET));
        assert!(layout_block(0, -2 * b, &params, &frame).contains(CellFlags::IS_STREET));
        assert!(!layout_block(-2, -2, &params, &frame).contains(CellFlags::IS_STREET));
    }

    #[test]
    fn block_layout_is_deterministic() {
        let params = zone_defaults(ZoneType::Residential);
        let frame = DistrictFrame::identity();
        for (x, y) in [(-17, 3), (0, 0), (255, 99), (1000, -1000)] {
            assert_eq!(
                layout_block(x, y, &params, &frame),
                layout_block(x, y, &params, &frame)
            );
        }
    }

    #[test]
    fn block_size_is_clamped_from_zero() {
        let params = ZoneParams {
            height_min: 0.0,
            height_max: 1.0,
            density: 1.0,
            block_size: 0,
            palette_count: 1,
            arterial_every: 0,
        };
        // block_size 0 must not panic; treat as 1 (every cell is a street).
        assert!(
            layout_block(0, 3, &params, &DistrictFrame::identity()).contains(CellFlags::IS_STREET)
        );
        assert!(
            layout_block(1, 3, &params, &DistrictFrame::identity()).contains(CellFlags::IS_STREET)
        );
    }

    #[test]
    fn arterials_are_two_cells_wide_and_flagged() {
        // block 9, every 2nd street arterial: avenue bx%2==0 owns rx 0..=1.
        let params = ZoneParams {
            arterial_every: 2,
            ..zone_defaults(ZoneType::Commercial)
        };
        let frame = DistrictFrame::identity();
        let b = i64::from(params.block_size);
        // Avenue at bx=0: rx=1 is a street (wide row), flagged arterial.
        let wide = layout_block(1, 5, &params, &frame);
        assert!(wide.contains(CellFlags::IS_STREET));
        assert!(wide.contains(CellFlags::IS_ARTERIAL));
        // Non-avenue block bx=1: rx=1 is interior, not a street.
        let local = layout_block(b + 1, 5, &params, &frame);
        assert_eq!(local, CellFlags::NONE);
        // Boundary row itself on an avenue is arterial too.
        let edge = layout_block(0, 5, &params, &frame);
        assert!(edge.contains(CellFlags::IS_ARTERIAL));
    }

    #[test]
    fn rotated_frame_shifts_the_grid() {
        // A 24° frame must move street membership for some cell (fabrics differ).
        let params = downtown_no_arterial();
        let plain = DistrictFrame::identity();
        let rotated = DistrictFrame {
            angle_rad: 24.0 * std::f64::consts::PI / 180.0,
            ..DistrictFrame::identity()
        };
        let mut differs = false;
        for x in 0..30 {
            for y in 0..30 {
                if layout_block(x, y, &params, &plain) != layout_block(x, y, &params, &rotated) {
                    differs = true;
                }
            }
        }
        assert!(differs, "rotated frame never changes the grid");
    }
}
