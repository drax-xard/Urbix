//! # lot.rs
//!
//! Block and lot subdivision for the Urbix engine (Milestone 11).
//!
//! The city reads correctly at eye level only when buildings are coherent at
//! lot scale (roughly 12–24 m), not white noise per cell. This module derives
//! the deterministic hierarchy **district → block → lot** that `street.rs`
//! (grid), `building.rs` (per-lot height/palette), and `chunk.rs`
//! (orchestration, lot-keyed interiors) all share.
//!
//! ## Responsibilities
//!
//! - `DistrictFrame`: per-district street orientation (quantized rotation) and
//!   warp (low-amplitude sine). Piecewise constant per district so fabrics
//!   differ and meet with intentional jogs.
//! - `block_loc`: map an absolute world cell to its block indices `(bx, bz)`
//!   and in-block offsets `(rx, rz)` in the district's rotated/warped frame.
//! - `lot_slot` / `lot_id_for`: split each block interior into 1–6
//!   street-facing lots along one axis; one stable `lot_id` per lot.
//! - `block_noise`: per-block correlated draw for density clumping and
//!   skyline variation (replaces per-cell salt-and-pepper vacancy).
//!
//! ## Determinism
//!
//! Every function is pure over **absolute** world coordinates plus
//! `(seed, domain)`. The same world cell always maps to the same block and
//! lot no matter which chunk asks, which is what keeps block edges aligned
//! across chunk boundaries. Rotation uses `round()` quantization of the local
//! frame; the rounding is deterministic and identical for every caller.

use crate::hash::{domain, hash_coords, hash_unit};

/// Per-district street orientation frame.
///
/// `angle_rad` is quantized to `{0°, ±12°, ±24°}` so neighbouring fabrics
/// meet with legible jogs rather than slivers. `warp_amp` (0.5–1.5 cells)
/// and `warp_len` (40–80 cells) bend the grid gently; `warp_phase` decorrelates
/// districts. The identity frame reproduces the legacy axis-aligned grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistrictFrame {
    /// Street rotation in radians (quantized, see above).
    pub angle_rad: f64,
    /// Sine warp amplitude in cells.
    pub warp_amp: f64,
    /// Sine warp wavelength in cells.
    pub warp_len: f64,
    /// Sine warp phase in radians.
    pub warp_phase: f64,
}

impl DistrictFrame {
    /// The legacy axis-aligned frame: no rotation, no warp.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::lot::DistrictFrame;
    /// let (lx, lz) = DistrictFrame::identity().transform(3, 7);
    /// assert!((lx - 3.0).abs() < 1e-9 && (lz - 7.0).abs() < 1e-9);
    /// ```
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            angle_rad: 0.0,
            warp_amp: 0.0,
            warp_len: 64.0,
            warp_phase: 0.0,
        }
    }

    /// Map absolute world cell coordinates into the district's local frame.
    ///
    /// Applies the sine warp first (in world space, so warps stay smooth
    /// across blocks) then rotates by `-angle` about the origin. The result
    /// is in "local cells" (fractional); callers quantize with `round()`
    /// before running the integer block grid.
    #[must_use]
    pub fn transform(&self, world_x: i64, world_z: i64) -> (f64, f64) {
        let x = world_x as f64;
        let z = world_z as f64;
        let (wx, wz) = if self.warp_amp > 0.0 {
            let two_pi = std::f64::consts::TAU;
            let ax = self.warp_amp * (two_pi * z / self.warp_len + self.warp_phase).sin();
            let az = self.warp_amp * (two_pi * x / self.warp_len + self.warp_phase * 1.7).sin();
            (x + ax, z + az)
        } else {
            (x, z)
        };
        // Rotate by -angle: local = Rot(-a) * world.
        let (s, c) = self.angle_rad.sin_cos();
        (c * wx + s * wz, -s * wx + c * wz)
    }
}

/// A cell's position inside the block lattice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockLoc {
    /// Block index along the local x axis (`div_euclid`, sign-stable).
    pub bx: i64,
    /// Block index along the local z axis.
    pub bz: i64,
    /// In-block offset `0..block_size` along x (`rem_euclid`).
    pub rx: i64,
    /// In-block offset `0..block_size` along z.
    pub rz: i64,
}

/// Locate a world cell in the block lattice for `block_size` + `frame`.
///
/// Quantizes the transformed frame with `round()` so rotated grids still land
/// on integer cells deterministically. A `block_size` of 0 is treated as 1.
///
/// ## Example
///
/// ```
/// use urbix::lot::{DistrictFrame, block_loc};
/// let loc = block_loc(11, 3, 8, &DistrictFrame::identity());
/// assert_eq!((loc.bx, loc.rx), (1, 3));
/// ```
#[must_use]
pub fn block_loc(world_x: i64, world_y: i64, block_size: u8, frame: &DistrictFrame) -> BlockLoc {
    let b = i64::from(block_size.max(1));
    let (lx, lz) = frame.transform(world_x, world_y);
    let cx = lx.round() as i64;
    let cz = lz.round() as i64;
    BlockLoc {
        bx: cx.div_euclid(b),
        bz: cz.div_euclid(b),
        rx: cx.rem_euclid(b),
        rz: cz.rem_euclid(b),
    }
}

/// A lot's identity inside its block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LotSlot {
    /// Stable lot key: unique per `(block, slot)`, shared by every cell in
    /// the lot. Folded as `bx * 8 + slot` so neighbouring blocks never
    /// collide (slot < 8 always).
    pub lot_id: u64,
    /// Index of this lot within the block (`0..count`).
    pub slot: u8,
    /// Number of lots in the block.
    pub count: u8,
    /// Whether the lot touches a street on two axes (first/last strip, or a
    /// single-lot block). Corner lots earn a height bonus in `building.rs`.
    pub corner: bool,
}

/// Split a block interior into street-facing lots and identify this cell's lot.
///
/// The buildable interior is `1..block_size` on each axis (offset 0 is the
/// street). Lots are 1-D strips along one axis — every lot keeps street
/// frontage, which is what makes ground floors walkable. The split axis and
/// count derive from the block hash (`domain::LOT_SPLIT`) so they are stable
/// per block:
///
/// - `count = clamp(buildable / 4, 1, 6)` (a 10-cell frontage yields 2 lots).
/// - Axis alternates per block so neighbouring blocks vary.
/// - `slot = pos * count / buildable` along the split axis.
///
/// Small blocks (`block_size <= 5`) stay single-lot.
///
/// ## Example
///
/// ```
/// use urbix::lot::{DistrictFrame, block_loc, lot_slot};
/// let frame = DistrictFrame::identity();
/// let loc = block_loc(3, 3, 11, &frame);
/// let lot = lot_slot(&loc, 11, 445566);
/// assert!(lot.slot < lot.count && lot.count >= 1);
/// ```
#[must_use]
pub fn lot_slot(loc: &BlockLoc, block_size: u8, seed: u64) -> LotSlot {
    let b = i64::from(block_size.max(1));
    if b <= 5 {
        // Tiny block: one lot touching every street — trivially a corner.
        let lot_id = hash_coords(loc.bx.wrapping_mul(8), loc.bz, seed, domain::LOT_SPLIT);
        return LotSlot {
            lot_id,
            slot: 0,
            count: 1,
            corner: true,
        };
    }
    // Buildable run excludes the street row at offset 0.
    let buildable = b - 1;
    let axis_roll = hash_unit(loc.bx, loc.bz, seed, domain::LOT_SPLIT);
    let along_x = axis_roll < 0.5;
    let pos = if along_x {
        (loc.rx - 1).clamp(0, buildable - 1)
    } else {
        (loc.rz - 1).clamp(0, buildable - 1)
    };
    let count = ((buildable / 4).clamp(1, 6)) as u8;
    let slot = ((pos * i64::from(count) / buildable).clamp(0, i64::from(count) - 1)) as u8;
    let lot_id = hash_coords(
        loc.bx.wrapping_mul(8).wrapping_add(i64::from(slot)),
        loc.bz.wrapping_add(if along_x { 0 } else { 1009 }),
        seed,
        domain::LOT_SPLIT,
    );
    LotSlot {
        lot_id,
        slot,
        count,
        corner: slot == 0 || slot + 1 == count,
    }
}

/// Per-block correlated draw in `[0, 1)`.
///
/// Used for density clumping (a whole lot lives or dies together) and
/// block-scale height variation. Distinct domain from lot splits so the two
/// never correlate spuriously.
///
/// ## Example
///
/// ```
/// use urbix::lot::block_noise;
/// let t = block_noise(2, -5, 445566);
/// assert!((0.0..1.0).contains(&t));
/// ```
#[must_use]
pub fn block_noise(bx: i64, bz: i64, seed: u64) -> f32 {
    hash_unit(bx, bz, seed, domain::BLOCK_NOISE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_frame_preserves_grid() {
        let f = DistrictFrame::identity();
        let loc = block_loc(11, 3, 8, &f);
        assert_eq!((loc.bx, loc.rx), (1, 3));
        assert_eq!((loc.bz, loc.rz), (0, 3));
    }

    #[test]
    fn block_loc_is_sign_stable() {
        // Negative worlds stay consistent (div/rem_euclid, never truncation).
        let f = DistrictFrame::identity();
        for (x, z) in [(-17, 3), (-1, -1), (-9, -16)] {
            let a = block_loc(x, z, 10, &f);
            let b = block_loc(x, z, 10, &f);
            assert_eq!(a, b);
            assert!((0..10).contains(&a.rx) && (0..10).contains(&a.rz));
        }
    }

    #[test]
    fn lot_ids_are_stable_per_lot_and_vary_across_lots() {
        let f = DistrictFrame::identity();
        // Same lot twice: identical keys (deterministic).
        let a = lot_slot(&block_loc(2, 5, 11, &f), 11, 99);
        let b = lot_slot(&block_loc(2, 5, 11, &f), 11, 99);
        assert_eq!(a.lot_id, b.lot_id);
        assert_eq!(a.slot, b.slot);
        // Somewhere in the neighbourhood the lot changes (scan both axes
        // since the split axis is hashed per block).
        let mut varied = false;
        for x in 0..24 {
            for z in 0..24 {
                let c = lot_slot(&block_loc(x, z, 11, &f), 11, 99);
                if c.lot_id != a.lot_id {
                    varied = true;
                }
            }
        }
        assert!(varied, "no lot variation across a 24x24 sample");
    }

    #[test]
    fn small_blocks_stay_single_lot() {
        let f = DistrictFrame::identity();
        let lot = lot_slot(&block_loc(1, 1, 4, &f), 4, 7);
        assert_eq!((lot.slot, lot.count), (0, 1));
        assert!(lot.corner);
    }

    #[test]
    fn lot_count_stays_bounded() {
        let f = DistrictFrame::identity();
        for b in [6u8, 9, 11, 14, 18, 32] {
            for x in 0..40 {
                let lot = lot_slot(&block_loc(x, 5, b, &f), b, 1234);
                assert!((1..=6).contains(&lot.count), "b={b} count={}", lot.count);
                assert!(lot.slot < lot.count, "b={b}");
            }
        }
    }

    #[test]
    fn block_noise_is_deterministic_unit() {
        let t = block_noise(2, -5, 445566);
        assert!((0.0..1.0).contains(&t));
        assert_eq!(t, block_noise(2, -5, 445566));
    }
}
