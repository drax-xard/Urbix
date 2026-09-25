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

//! # street.rs
//!
//! Street grid and block subdivision for the Urbix engine.
//!
//! Milestone 11 replaces the single global graph-paper grid with a
//! **varied fabric**: each district owns an orientation frame
//! (`lot::DistrictFrame` — quantized rotation plus sine warp), and streets
//! form a two-tier hierarchy — 1-cell local streets plus 2-cell arterial
//! avenues every `ZoneParams::arterial_every` streets.
//!
//! Milestone 12 breaks the remaining monotony three ways: local street
//! segments drop out by hash (~8%, arterials exempt), merging neighbours
//! into superblocks; dropped segments mostly reopen as linear-park
//! greenways; and two global diagonal boulevards cut across every grid.
//!
//! ## Responsibilities
//!
//! - `street_info`: the full per-cell answer — grid street, arterial,
//!   diagonal boulevard, dropped segment, greenway — for `chunk.rs`.
//! - `layout_block`: the legacy base-lattice query (grid + arterials only,
//!   no diagonals or dropout), kept for unit tests of the lattice and as a
//!   cheap neighbour check.
//! - Sidewalk rings, plazas, and block interiors are composed in `chunk.rs`
//!   on top of this query.
//! - Flow avenues (Milestone 16) are deliberately NOT resolved here: they are
//!   frame-independent world-space segments owned by `region.rs`, so
//!   `chunk.rs` ORs them onto this answer (and into the sidewalk check)
//!   instead of threading them through the district frame.
//!
//! ## Determinism
//!
//! The layout is computed from **absolute** world cell coordinates in the
//! district frame, so the same cell is a street (or not) regardless of which
//! chunk asks. Dropout is keyed on absolute block-boundary segments, and
//! diagonals live in world coordinates, so both stay seamless across chunks.

use crate::data::CellFlags;
use crate::hash::{domain, hash_unit};
use crate::lot::{block_loc, Diagonal, DistrictFrame};
use crate::zones::ZoneParams;

/// Share of local (non-arterial) street segments that drop out, merging
/// neighbours into superblocks. Arterials never drop, so the city stays
/// permeable at the avenue scale.
const DROP_SHARE: f32 = 0.08;

/// Share of dropped segments that reopen as linear-park greenways (the rest
/// merge into the adjoining block interior).
const GREENWAY_SHARE: f32 = 0.6;

/// Full per-cell street answer for `chunk.rs` orchestration.
///
/// All fields derive from absolute world coordinates plus `(seed, domain)`,
/// so every caller agrees on every cell:
///
/// - `street`: the cell is paved road (grid, arterial, or boulevard).
/// - `arterial`: the street is an avenue-scale road (arterial lattice or
///   diagonal boulevard).
/// - `diagonal`: the cell lies on a global diagonal boulevard.
/// - `dropped`: a local grid segment removed by hash (superblock merge).
/// - `greenway`: a dropped segment reborn as linear park (`dropped` implies
///   exactly one of `greenway` / merged-interior).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreetInfo {
    /// Paved road of any kind.
    pub street: bool,
    /// Avenue-scale road.
    pub arterial: bool,
    /// On a global diagonal boulevard.
    pub diagonal: bool,
    /// Local segment removed by hash.
    pub dropped: bool,
    /// Dropped segment kept as linear park.
    pub greenway: bool,
}

/// Answer the full street question for a world cell.
///
/// `diagonals` are the region's global boulevards (`region.rs`); pass an
/// empty slice for the pure lattice. `seam_road` marks district-bisector
/// cells (`VoronoiDiagram::is_seam_road`): the boundary parkway both grids
/// tee into, immune to dropout. Dropout applies only to local grid streets
/// (never arterials, boulevards, or seams — the caller re-adds plazas
/// afterwards).
///
/// ## Example
///
/// ```
/// use urbix::street::{street_info, StreetInfo};
/// use urbix::lot::DistrictFrame;
/// use urbix::zones::{ZoneType, zone_defaults};
///
/// let params = zone_defaults(ZoneType::Downtown);
/// let frame = DistrictFrame::identity();
/// let info = street_info(0, 0, &params, &frame, &[], false, 445566);
/// assert!(info.street);
/// ```
#[must_use]
#[allow(clippy::too_many_arguments)] // one bundle per hierarchy level; the pipeline passes them through
pub fn street_info(
    cell_x: i64,
    cell_y: i64,
    params: &ZoneParams,
    frame: &DistrictFrame,
    diagonals: &[Diagonal],
    seam_road: bool,
    seed: u64,
) -> StreetInfo {
    // Boulevards first: world-space lines, 2 cells wide, always arterial.
    let mut diagonal = false;
    for d in diagonals {
        if d.covers(cell_x, cell_y) {
            diagonal = true;
            break;
        }
    }

    // Seam parkway: the district border itself, paved as an avenue so both
    // grids terminate at a real road instead of tearing.
    if seam_road {
        return StreetInfo {
            street: true,
            arterial: true,
            diagonal,
            dropped: false,
            greenway: false,
        };
    }

    let loc = block_loc(cell_x, cell_y, params.block_size, frame);
    let k = i64::from(params.arterial_every);
    let avenue_v = k >= 2 && loc.bx.rem_euclid(k) == 0;
    let avenue_h = k >= 2 && loc.bz.rem_euclid(k) == 0;
    let on_avenue =
        (avenue_v && (loc.rx == 0 || loc.rx == 1)) || (avenue_h && (loc.rz == 0 || loc.rz == 1));
    let on_grid = loc.rx == 0 || loc.rz == 0;

    if on_avenue || diagonal {
        return StreetInfo {
            street: true,
            arterial: true,
            diagonal,
            dropped: false,
            greenway: false,
        };
    }
    if on_grid {
        // Dropout candidates: local boundary rows only (avenues returned
        // above). Streets drop in arterial-to-arterial stretches — keyed by
        // (boundary line, superblock band, orientation) — so a missing local
        // merges neighbours into one superblock and terminates at avenues as
        // T-junctions, instead of dashing the road into dead-ends. Park zones
        // (no arterials) fall back to 4-block bands.
        let band_k = if k >= 2 { k } else { 4 };
        // Corners key vertical for determinism.
        let (id_a, id_b) = if loc.rx == 0 {
            (loc.bx, loc.bz.div_euclid(band_k).wrapping_mul(2))
        } else {
            (
                loc.bz,
                loc.bx.div_euclid(band_k).wrapping_mul(2).wrapping_add(1),
            )
        };
        if hash_unit(id_a, id_b, seed, domain::STREET_DROP) < DROP_SHARE {
            let greenway = hash_unit(id_b, id_a, seed, domain::GREENWAY) < GREENWAY_SHARE;
            return StreetInfo {
                street: false,
                arterial: false,
                diagonal: false,
                dropped: true,
                greenway,
            };
        }
        return StreetInfo {
            street: true,
            arterial: false,
            diagonal: false,
            dropped: false,
            greenway: false,
        };
    }
    StreetInfo {
        street: false,
        arterial: false,
        diagonal: false,
        dropped: false,
        greenway: false,
    }
}

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
            slenderness_max: 0,
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

    #[test]
    fn dropout_merges_some_locals_but_never_arterials() {
        // Over a wide sample some local stretches drop (superblocks form),
        // while every arterial avenue cell stays paved.
        use crate::lot::Diagonal;
        let params = zone_defaults(ZoneType::Downtown); // block 11, K 4
        let frame = DistrictFrame::identity();
        let diags: &[Diagonal] = &[];
        let mut dropped = 0usize;
        let mut greenway = 0usize;
        let mut local_street = 0usize;
        for x in 0..220 {
            for z in 0..220 {
                let info = street_info(x, z, &params, &frame, diags, false, 445566);
                if info.dropped {
                    dropped += 1;
                }
                if info.greenway {
                    greenway += 1;
                    assert!(!info.street);
                }
                if info.street && !info.arterial {
                    local_street += 1;
                }
                // Arterial rows are never dropped: any cell the base lattice
                // flags arterial must still read street through street_info.
                let base = layout_block(x, z, &params, &frame);
                if base.contains(CellFlags::IS_ARTERIAL) {
                    assert!(info.street, "arterial dropped at ({x},{z})");
                    assert!(!info.dropped);
                }
            }
        }
        assert!(dropped > 0, "no dropout sampled");
        assert!(greenway > 0, "no greenways sampled");
        assert!(local_street > 0, "no local streets left");
        // Dropout is a minority texture, not the dominant fabric.
        let total = 220 * 220;
        assert!(
            dropped < total / 5,
            "dropout too aggressive: {dropped}/{total}"
        );
    }

    #[test]
    fn diagonals_pave_arterial_boulevards() {
        use crate::lot::Diagonal;
        let params = downtown_no_arterial();
        let frame = DistrictFrame::identity();
        // East-running boulevard through the origin, 2 cells wide.
        let diags = [Diagonal {
            angle_rad: 0.0,
            offset: 0.0,
            half_width: 1.0,
        }];
        // Cells on the line are arterial streets even mid-block.
        let mid = street_info(5, 0, &params, &frame, &diags, false, 7);
        assert!(mid.street && mid.arterial && mid.diagonal);
        let mid2 = street_info(5, 1, &params, &frame, &diags, false, 7);
        assert!(mid2.street && mid2.diagonal);
        // Far off the line, the lattice rules alone.
        let off = street_info(5, 30, &params, &frame, &diags, false, 7);
        assert!(!off.diagonal);
        // Empty diagonal set disables boulevards entirely.
        let empty: &[Diagonal] = &[];
        assert!(!street_info(5, 0, &params, &frame, empty, false, 7).diagonal);
    }

    #[test]
    fn street_info_is_deterministic() {
        use crate::lot::Diagonal;
        let params = zone_defaults(ZoneType::Commercial);
        let frame = DistrictFrame::identity();
        let diags = [Diagonal {
            angle_rad: 0.7,
            offset: 13.0,
            half_width: 1.0,
        }];
        for (x, z) in [(-40, 17), (0, 0), (300, -211)] {
            assert_eq!(
                street_info(x, z, &params, &frame, &diags, false, 99),
                street_info(x, z, &params, &frame, &diags, false, 99)
            );
        }
    }

    #[test]
    fn seam_cells_are_arterial_and_immune_to_dropout() {
        // A seam parkway paves even deep-interior cells and never drops.
        let params = zone_defaults(ZoneType::Downtown);
        let frame = DistrictFrame::identity();
        let diags: &[crate::lot::Diagonal] = &[];
        let info = street_info(4, 4, &params, &frame, diags, true, 445566);
        assert!(info.street && info.arterial);
        assert!(!info.dropped && !info.greenway);
        // Same cell without the seam flag is ordinary interior (block 11).
        let plain = street_info(4, 4, &params, &frame, diags, false, 445566);
        assert!(!plain.street);
    }
}
