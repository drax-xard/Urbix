//! # street.rs
//!
//! Street grid and block subdivision for the Urbix engine.
//!
//! This module decides which cells are part of the road network and how the
//! remaining space is divided into building blocks. Street layout is tuned per
//! zone: tight small blocks downtown, roomy blocks in residential areas, and
//! large wide blocks in industrial districts.
//!
//! ## Responsibilities
//!
//! - Given a cell's zone parameters, decide whether it is a street.
//! - Streets carry `height = 0` and are excluded from building placement.
//! - Delimits the interior of each block, which is then filled with a building
//!   footprint (or left open/park cells) by `building.rs`.
//!
//! ## Determinism
//!
//! The layout is computed from **absolute** world cell coordinates, so the same
//! cell is a street (or not) regardless of which chunk asks. This is what keeps
//! blocks aligned and consistent across chunk boundaries.
//!
//! ## Block grid
//!
//! A cell is a street when its world x or world y coordinate falls exactly on a
//! block boundary (`coord % block_size == 0`). This produces a regular street
//! grid where every `block_size`-th row and column is a road.

use crate::data::CellFlags;
use crate::zones::ZoneParams;

/// Decide whether a world cell lands on the street grid.
///
/// Returns `CellFlags::IS_STREET` (or empty flags) based on whether the cell's
/// absolute world coordinates fall on a block boundary for the zone's block
/// size. A block size of 0 is treated as 1 to avoid modulo-by-zero.
///
/// ## Example
///
/// ```
/// use urbix::street::layout_block;
/// use urbix::data::CellFlags;
/// use urbix::zones::{ZoneParams, ZoneType, zone_defaults};
///
/// let params = zone_defaults(ZoneType::Downtown); // block_size 4
/// assert!(layout_block(0, 0, &params).contains(CellFlags::IS_STREET));
/// assert!(layout_block(4, 5, &params).contains(CellFlags::IS_STREET));
/// assert!(!layout_block(1, 1, &params).contains(CellFlags::IS_STREET));
/// ```
#[must_use]
pub fn layout_block(cell_x: i32, cell_y: i32, params: &ZoneParams) -> CellFlags {
    let size = params.block_size.max(1) as i32;
    let on_vertical = cell_x.rem_euclid(size) == 0;
    let on_horizontal = cell_y.rem_euclid(size) == 0;
    if on_vertical || on_horizontal {
        CellFlags::IS_STREET
    } else {
        CellFlags::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zones::{zone_defaults, ZoneType};

    #[test]
    fn street_falls_on_block_boundaries() {
        let params = zone_defaults(ZoneType::Downtown); // block_size 4
        assert!(layout_block(0, 0, &params).contains(CellFlags::IS_STREET));
        assert!(layout_block(4, 0, &params).contains(CellFlags::IS_STREET));
        assert!(layout_block(0, 8, &params).contains(CellFlags::IS_STREET));
        assert!(layout_block(12, 3, &params).contains(CellFlags::IS_STREET));
        // Interior cells are not streets.
        assert_eq!(layout_block(1, 1, &params), CellFlags::NONE);
        assert_eq!(layout_block(3, 3, &params), CellFlags::NONE);
    }

    #[test]
    fn street_handles_negative_coordinates() {
        // rem_euclid keeps boundaries consistent across the sign change.
        let params = zone_defaults(ZoneType::Commercial); // block_size 5
        assert!(layout_block(-5, 2, &params).contains(CellFlags::IS_STREET));
        assert!(layout_block(5, 2, &params).contains(CellFlags::IS_STREET));
        assert!(layout_block(0, -10, &params).contains(CellFlags::IS_STREET));
        assert!(!layout_block(-2, -2, &params).contains(CellFlags::IS_STREET));
    }

    #[test]
    fn block_layout_is_deterministic() {
        let params = zone_defaults(ZoneType::Residential);
        for (x, y) in [(-17, 3), (0, 0), (255, 99), (1000, -1000)] {
            assert_eq!(layout_block(x, y, &params), layout_block(x, y, &params));
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
        };
        // block_size 0 must not panic; treat as 1 (every cell is a street).
        assert!(layout_block(0, 3, &params).contains(CellFlags::IS_STREET));
        assert!(layout_block(1, 3, &params).contains(CellFlags::IS_STREET));
    }
}
