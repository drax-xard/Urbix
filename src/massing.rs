//! # massing.rs
//!
//! Split-grammar building massing for the Urbix engine (Milestone 17, G0).
//!
//! Milestone 11 extrudes every lot as a flat rectangle; from the air the
//! skyline reads as stepped boxes of uniform height per lot. This module
//! splits each lot's mass into per-cell height steps — podium bases, pitched
//! house roofs, sawtooth shed bays, crown caps — so the five zones read as
//! five architectures while the wire still carries one `height` per cell.
//!
//! ## Responsibilities
//!
//! - `MassingParams`: the per-zone split-grammar rule table (one row per
//!   `ZoneType`, tuned via `WorldConfig` like the M9 blueprints).
//! - `massing_for`: the pure per-cell step function — podium cap, roof peak,
//!   crown boost — applied to a lot's base height `H`.
//! - `massing_defaults` / `default_massing`: the canonical per-zone table.
//!
//! ## Ordering (locked by `docs/grammar_massing.md` §2.2)
//!
//! `massing_for` computes podium → roof → crown only. The caller
//! (`building.rs`) applies the ±10% `HEIGHT` jitter *after* the steps and the
//! `1.3× band` + `slenderness_cap` clamps *last* (reused at both clamp sites),
//! so life survives on every mass and no step outgrows its lot.
//!
//! ## G0 scope (explicitly deferred)
//!
//! - No WFC streets, no facade dressing, no grammar↔interior bridge — height
//!   steps only. `setback_every` parses and validates but does nothing (the
//!   schema stays stable into G2 multi-tier towers).
//! - `MASS_PODIUM` is declared but unhashed in G0: the podium cap is an exact
//!   `podium_floors × floor_height` step, so per-lot podium articulation waits
//!   for G2 alongside `setback_every`.
//!
//! ## Determinism
//!
//! Every step is integer offset arithmetic plus seeded hashes keyed on
//! `lot_id` (stable per block+slot, never per chunk), so two chunks querying
//! the same world cell compute the same step and sawtooth bays/crowns align
//! across chunk borders by construction. No trig, no float sort, no libm
//! widening beyond what Shepard already uses.

use serde::{Deserialize, Serialize};

use crate::hash::{domain, hash_coords, hash_unit};
use crate::zones::{ZoneType, ZONE_COUNT};

/// Height of one cell in metres (the M11 scale canon: 1 cell = 4 m).
///
/// Floors the podium cap so a `podium_floors = 0` config can never zero an
/// edge ring (see [`massing_for`]).
const CELL_METRES: f32 = 4.0;

/// Pitched-roof peak multiplier on the ridge line.
const RIDGE_PEAK: f32 = 1.15;
/// Pitched-roof multiplier at the eaves.
const EAVES_PEAK: f32 = 0.90;
/// Sawtooth high-bay multiplier (one bay in three along x).
const SAW_HIGH: f32 = 1.12;
/// Sawtooth low-bay multiplier.
const SAW_LOW: f32 = 0.94;
/// Sawtooth bay period in cells.
const SAW_PERIOD: u64 = 3;
/// Crown-cap boost on inner cells of winning lots.
const CROWN_BOOST: f32 = 1.12;

/// Per-zone split-grammar massing rules (Milestone 17, G0).
///
/// A plain `#[repr(C)]`, `Serialize`/`Deserialize` row tuned per zone via
/// `WorldConfig` (same compromise `Blueprint` made in M9: five scalars, not
/// rule trees). All zeros (or `massing_mode = 0` at the config level)
/// reproduces the legacy flat extrude exactly — see [`is_flat`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct MassingParams {
    /// Full-footprint base height in floors (`0` = no podium split).
    pub podium_floors: u8,
    /// Edge rings from the lot border capped to the podium (`0` = flat).
    pub tower_inset: u8,
    /// Upper-mass step-back rhythm. **Reserved in G0**: accepted, validated,
    /// and ignored — it wires in G2, when per-floor massing exists.
    pub setback_every: u8,
    /// Roof shape: `0` flat, `1` pitched (ridge along the long axis),
    /// `2` sawtooth (bays along x). Unknown values read as flat.
    pub roof_mode: u8,
    /// Share of lots earning a crown cap on inner cells (`0..=1`).
    pub crown_share: f32,
}

/// Default massing rules for a single [`ZoneType`].
///
/// The canonical G0 table from `docs/grammar_massing.md`: towers on podiums
/// downtown, shopfront bases in commercial, ridged houses residential,
/// banded sheds industrial, flat sheds in parks.
///
/// ## Example
///
/// ```
/// use urbix::massing::massing_defaults;
/// use urbix::zones::ZoneType;
///
/// let dt = massing_defaults(ZoneType::Downtown);
/// assert_eq!((dt.podium_floors, dt.tower_inset, dt.roof_mode), (3, 1, 0));
/// let res = massing_defaults(ZoneType::Residential);
/// assert_eq!(res.roof_mode, 1);
/// ```
#[must_use]
pub const fn massing_defaults(zone: ZoneType) -> MassingParams {
    match zone {
        ZoneType::Downtown => MassingParams {
            podium_floors: 3,
            tower_inset: 1,
            setback_every: 0,
            roof_mode: 0,
            crown_share: 0.3,
        },
        ZoneType::Residential => MassingParams {
            podium_floors: 0,
            tower_inset: 0,
            setback_every: 0,
            roof_mode: 1,
            crown_share: 0.0,
        },
        ZoneType::Commercial => MassingParams {
            podium_floors: 2,
            tower_inset: 1,
            setback_every: 0,
            roof_mode: 0,
            crown_share: 0.15,
        },
        ZoneType::Industrial => MassingParams {
            podium_floors: 0,
            tower_inset: 0,
            setback_every: 0,
            roof_mode: 2,
            crown_share: 0.0,
        },
        ZoneType::Park => MassingParams {
            podium_floors: 0,
            tower_inset: 0,
            setback_every: 0,
            roof_mode: 0,
            crown_share: 0.0,
        },
    }
}

/// The default per-zone massing table (index = `ZoneType` discriminant).
///
/// ## Example
///
/// ```
/// use urbix::massing::default_massing;
/// use urbix::zones::{ZoneType, ZONE_COUNT};
///
/// let table = default_massing();
/// assert_eq!(table.len(), ZONE_COUNT);
/// assert_eq!(table[ZoneType::Downtown as usize].tower_inset, 1);
/// ```
#[must_use]
pub fn default_massing() -> [MassingParams; ZONE_COUNT] {
    [
        massing_defaults(ZoneType::Downtown),
        massing_defaults(ZoneType::Residential),
        massing_defaults(ZoneType::Commercial),
        massing_defaults(ZoneType::Industrial),
        massing_defaults(ZoneType::Park),
    ]
}

/// Whether these rules leave the base height untouched.
///
/// A flat table (no podium, no inset, flat roof, no crowns) makes
/// [`massing_for`] the identity — the legacy byte-identical anchor.
/// `setback_every` is ignored: it is reserved, never wired in G0.
#[must_use]
pub const fn is_flat(params: &MassingParams) -> bool {
    params.podium_floors == 0
        && params.tower_inset == 0
        && params.roof_mode == 0
        && params.crown_share <= 0.0
}

/// Inset ring index of a cell inside its lot rect (`0` = lot edge).
///
/// Pure offset arithmetic on clamped geometry (same clamps as
/// [`massing_for`]), so examples and gates recompute exactly what generation
/// used without duplicating the pipeline.
#[must_use]
pub fn massing_ring(lot_rect: (u8, u8), cell_offset: (u8, u8)) -> u8 {
    let w = i32::from(lot_rect.0).max(1);
    let d = i32::from(lot_rect.1).max(1);
    let ox = i32::from(cell_offset.0).clamp(0, w - 1);
    let oz = i32::from(cell_offset.1).clamp(0, d - 1);
    ox.min(oz).min(w - 1 - ox).min(d - 1 - oz) as u8
}

/// Podium cap for these rules in metres (`podium_floors × floor_height`,
/// floored at one cell so a zero-podium config never zeroes an edge ring).
#[must_use]
pub fn podium_top(params: &MassingParams, floor_height: f32) -> f32 {
    (f32::from(params.podium_floors) * floor_height).max(CELL_METRES)
}

/// Whether this lot earns the crown cap on its inner cells.
///
/// The `MASS_CROWN` roll falls under `crown_share`; a zero share never
/// crowns. Deterministic per `(lot_id, seed)`.
#[must_use]
pub fn lot_crowns(lot_id: u64, params: &MassingParams, seed: u64) -> bool {
    if params.crown_share <= 0.0 {
        return false;
    }
    hash_unit(
        lot_id as i64,
        (lot_id >> 32) as i64,
        seed,
        domain::MASS_CROWN,
    ) < params.crown_share
}

/// Shape one cell's share of a lot's mass.
///
/// Takes the lot's base height `H` (the existing band → clump → CBD →
/// corner pipeline) and returns the stepped height for the cell at
/// `cell_offset` inside `lot_rect`, both in cells. The caller still applies
/// the `HEIGHT` jitter and the band/slenderness caps afterwards.
///
/// Steps, in order:
///
/// 1. **Podium split** (`tower_inset > 0`): edge rings cap at
///    `podium_top = podium_floors × floor_height` (floored at one cell so a
///    zero-podium config never zeroes); inner cells keep `H`. Lots no taller
///    than their podium (`H <= podium_top`) return `H` untouched — the
///    spec's single-storey no-op, which also keeps crowns off stub buildings.
/// 2. **Roof**: pitched ridges along the long axis (`1.15` → `0.90`
///    ridge-to-eaves, pure offset arithmetic); sawtooth alternates
///    `1.12 / 0.94` bands along x with period 3 from a `lot_id`-keyed phase.
/// 3. **Crown** (`crown_share > 0`): lots whose `MASS_CROWN` roll falls under
///    the share scale inner cells (`ring >= tower_inset`) by `1.12`. Edge
///    cells never crown, so street walls stay capped.
///
/// Offsets outside the rect are clamped to it (never panics); `floor_height`
/// is `WorldConfig::interior_floor_height` (1 cell per storey at the default).
///
/// ## Example
///
/// ```
/// use urbix::massing::{massing_defaults, massing_for};
/// use urbix::zones::ZoneType;
///
/// let dt = massing_defaults(ZoneType::Downtown); // podium 3 × 4 m = 12 m
/// // Edge ring caps at the podium, the core keeps full height.
/// assert_eq!(massing_for(7, (7, 7), (0, 3), 60.0, &dt, 69420, 4.0), 12.0);
/// ```
#[must_use]
pub fn massing_for(
    lot_id: u64,
    lot_rect: (u8, u8),
    cell_offset: (u8, u8),
    base_height: f32,
    params: &MassingParams,
    seed: u64,
    floor_height: f32,
) -> f32 {
    // Cheap legacy path: a flat table is the identity (bit-exact: no math).
    if is_flat(params) {
        return base_height;
    }
    // Defensive geometry: degenerate rects collapse to 1×1, stray offsets
    // clamp inside — pure integer math, chunk-edge safe, never panics.
    let w = i32::from(lot_rect.0).max(1);
    let d = i32::from(lot_rect.1).max(1);
    let ox = i32::from(cell_offset.0).clamp(0, w - 1);
    let oz = i32::from(cell_offset.1).clamp(0, d - 1);
    // Inset ring index: 0 = lot edge (shared helper, see `massing_ring`).
    let ring = massing_ring(lot_rect, cell_offset);

    // Podium cap: edge rings hold the street wall at podium height.
    let top = podium_top(params, floor_height);
    // Single-storey lots never reach past their podium: untouched, by
    // construction (no crown on stubs, no roof on sub-podium heights).
    if base_height <= top {
        return base_height;
    }
    let mut height = if ring < params.tower_inset {
        base_height.min(top)
    } else {
        base_height
    };

    // Roof shape: ridge along the lot's long axis, or sawtooth bays.
    height *= roof_peak(lot_id, ox, oz, w, d, params.roof_mode, seed);

    // Crown cap: a hashed share of lots rises 12% on inner cells only.
    if ring >= params.tower_inset && lot_crowns(lot_id, params, seed) {
        height *= CROWN_BOOST;
    }
    height
}

/// Roof multiplier for one cell: pitched ridge, sawtooth bays, or flat.
///
/// Pitched (`1`) ridges along the long axis: `1.15` on the ridge line
/// falling linearly to `0.90` at the eaves (offset ratios only — no trig).
/// Sawtooth (`2`) bands along x with period 3: `(offset + phase) % 3 == 0`
/// bays read high (`1.12`, the skylight slope), the rest low (`0.94`); the
/// phase derives from `lot_id`, so bays align across chunk borders. Flat
/// (`0`) and unknown modes return `1.0`.
fn roof_peak(lot_id: u64, ox: i32, oz: i32, w: i32, d: i32, roof_mode: u8, seed: u64) -> f32 {
    match roof_mode {
        1 => {
            // Ridge along the long axis; fall off across the short axis.
            let (off, span) = if w >= d {
                (oz as f32, d as f32)
            } else {
                (ox as f32, w as f32)
            };
            let half = (span - 1.0) / 2.0;
            if half <= 0.0 {
                RIDGE_PEAK
            } else {
                RIDGE_PEAK - (RIDGE_PEAK - EAVES_PEAK) * ((off - half).abs() / half)
            }
        }
        2 => {
            let phase = hash_coords(
                lot_id as i64,
                (lot_id >> 32) as i64,
                seed,
                domain::MASS_ROOF,
            ) % SAW_PERIOD;
            if (ox as u64 + phase).is_multiple_of(SAW_PERIOD) {
                SAW_HIGH
            } else {
                SAW_LOW
            }
        }
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sweep helper: fraction of `lot_id`s in `0..n` whose inner cell crowns.
    fn crown_fraction(n: u64, params: &MassingParams, seed: u64) -> f64 {
        let mut crowned = 0;
        for lot_id in 0..n {
            // Multi-storey lot well above any podium, inner cell.
            let h = massing_for(lot_id, (7, 7), (3, 3), 60.0, params, seed, 4.0);
            if h > 60.0 {
                crowned += 1;
            }
        }
        crowned as f64 / n as f64
    }

    #[test]
    fn flat_table_is_identity() {
        let flat = MassingParams {
            podium_floors: 0,
            tower_inset: 0,
            setback_every: 0,
            roof_mode: 0,
            crown_share: 0.0,
        };
        assert!(is_flat(&flat));
        // Bit-exact across rects, offsets, and seeds (no math runs).
        for &(rect, off) in &[((7, 7), (0, 3)), ((7, 7), (3, 3)), ((4, 9), (2, 8))] {
            for seed in [1, 69420, u64::MAX] {
                assert_eq!(massing_for(7, rect, off, 60.0, &flat, seed, 4.0), 60.0);
            }
        }
        // Non-flat tables are not flat (each live knob trips it alone).
        assert!(!is_flat(&massing_defaults(ZoneType::Downtown)));
        assert!(!is_flat(&massing_defaults(ZoneType::Residential)));
        assert!(!is_flat(&massing_defaults(ZoneType::Industrial)));
        assert!(!is_flat(&MassingParams {
            roof_mode: 9,
            ..flat
        }));
    }

    #[test]
    fn massing_is_deterministic() {
        let dt = massing_defaults(ZoneType::Downtown);
        let a = massing_for(0x9e37_79b9_7f4a_7c15, (7, 7), (1, 5), 60.0, &dt, 69420, 4.0);
        let b = massing_for(0x9e37_79b9_7f4a_7c15, (7, 7), (1, 5), 60.0, &dt, 69420, 4.0);
        assert_eq!(a, b);
        // Different lots / seeds vary (crown + phase rolls are live).
        let mut seen = std::collections::HashSet::new();
        for lot_id in 0..50 {
            seen.insert(massing_for(lot_id, (7, 7), (3, 3), 60.0, &dt, 69420, 4.0).to_bits());
        }
        assert!(seen.len() > 1, "crown never varies");
    }

    #[test]
    fn podium_ring_shorter_than_tower_core() {
        // Exact heights with crowns off: edge caps at 3 floors × 4 m.
        let no_crown = MassingParams {
            crown_share: 0.0,
            ..massing_defaults(ZoneType::Downtown)
        };
        assert_eq!(
            massing_for(7, (7, 7), (0, 3), 60.0, &no_crown, 69420, 4.0),
            12.0
        );
        assert_eq!(
            massing_for(7, (7, 7), (3, 3), 60.0, &no_crown, 69420, 4.0),
            60.0
        );
        // With live crowns the ordering still holds on every lot: the edge
        // never crowns, the core never sinks to the podium.
        let dt = massing_defaults(ZoneType::Downtown);
        for lot_id in 0..200 {
            let edge = massing_for(lot_id, (7, 7), (0, 3), 60.0, &dt, 69420, 4.0);
            let core = massing_for(lot_id, (7, 7), (3, 3), 60.0, &dt, 69420, 4.0);
            assert_eq!(edge, 12.0, "edge must hold the podium exactly");
            assert!(core >= 60.0, "core must keep full height or crown");
        }
    }

    #[test]
    fn single_storey_lots_unaffected() {
        // H below the podium top returns untouched — no podium cut, no roof,
        // no crown — even with every knob live (downtown crown 0.30).
        let dt = massing_defaults(ZoneType::Downtown);
        for lot_id in 0..50 {
            for &(ox, oz) in &[(0, 0), (0, 3), (3, 3), (6, 6)] {
                assert_eq!(
                    massing_for(lot_id, (7, 7), (ox, oz), 8.0, &dt, lot_id, 4.0),
                    8.0,
                    "stub lot {lot_id} cell ({ox}, {oz}) moved"
                );
            }
        }
    }

    #[test]
    fn crown_share_matches_config() {
        // Statistical, seed-agnostic: sweep lots, assert the crowned share
        // lands in a band around the configured share (never a hardcoded seed).
        let dt = massing_defaults(ZoneType::Downtown); // 0.30
        let frac = crown_fraction(2000, &dt, 69420);
        assert!(
            (0.20..=0.40).contains(&frac),
            "downtown crown share {frac} off 0.30"
        );
        let com = massing_defaults(ZoneType::Commercial); // 0.15
        let frac = crown_fraction(2000, &com, 12345);
        assert!(
            (0.08..=0.22).contains(&frac),
            "commercial crown share {frac} off 0.15"
        );
        // Zero share never crowns, anywhere.
        let park = massing_defaults(ZoneType::Park);
        assert_eq!(crown_fraction(500, &park, 69420), 0.0);
    }

    #[test]
    fn pitched_ridge_taller_than_eaves() {
        let res = massing_defaults(ZoneType::Residential); // pitched, no crown
                                                           // 7 × 5 lot: long axis x, ridge at oz = 2.
        let ridge = massing_for(7, (7, 5), (3, 2), 20.0, &res, 69420, 4.0);
        let eaves = massing_for(7, (7, 5), (3, 0), 20.0, &res, 69420, 4.0);
        assert!(ridge > eaves, "ridge {ridge} must beat eaves {eaves}");
        let ratio = ridge / eaves;
        assert!(
            (1.25..=1.31).contains(&ratio),
            "ridge/eaves {ratio} off 1.15/0.90 ≈ 1.278"
        );
        // Symmetric lots read symmetric: mirrored eaves cells agree exactly
        // (same offset magnitude, no hash in the pitched path).
        assert_eq!(
            massing_for(7, (7, 5), (3, 0), 20.0, &res, 69420, 4.0),
            massing_for(7, (7, 5), (3, 4), 20.0, &res, 69420, 4.0)
        );
        // Long axis flips with the rect: 5 × 7 ridges along z instead.
        let ridge_z = massing_for(7, (5, 7), (2, 3), 20.0, &res, 69420, 4.0);
        let eaves_z = massing_for(7, (5, 7), (0, 3), 20.0, &res, 69420, 4.0);
        assert!(ridge_z > eaves_z);
    }

    #[test]
    fn sawtooth_has_period_three() {
        let ind = massing_defaults(ZoneType::Industrial); // sawtooth, no crown
                                                          // Period 3 along x: ox and ox+3 always agree (same phase, same band).
        for lot_id in [7, 1234, 999_999] {
            for ox in 0..6 {
                let a = massing_for(lot_id, (9, 6), (ox, 2), 15.0, &ind, 69420, 4.0);
                let b = massing_for(lot_id, (9, 6), (ox + 3, 2), 15.0, &ind, 69420, 4.0);
                assert_eq!(a, b, "period broken at ox={ox}");
            }
            // Both bay levels appear within any 3-run (high bays exist).
            let mut levels = std::collections::HashSet::new();
            for ox in 0..3 {
                levels
                    .insert(massing_for(lot_id, (9, 6), (ox, 2), 15.0, &ind, 69420, 4.0).to_bits());
            }
            assert_eq!(levels.len(), 2, "expected high + low bays");
        }
    }

    #[test]
    fn reserved_setback_every_is_noop() {
        // G0→G2 schema stability: nonzero setback_every parses but changes
        // nothing (covered for every zone default + a crowned case).
        for zone in ZoneType::all() {
            let base = massing_defaults(zone);
            let reserved = MassingParams {
                setback_every: 7,
                ..base
            };
            for lot_id in 0..50 {
                for &(ox, oz) in &[(0, 0), (1, 2), (3, 3)] {
                    assert_eq!(
                        massing_for(lot_id, (7, 7), (ox, oz), 60.0, &base, 99, 4.0),
                        massing_for(lot_id, (7, 7), (ox, oz), 60.0, &reserved, 99, 4.0),
                        "setback_every leaked into {zone:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn unknown_roof_mode_reads_flat() {
        // Defensive: out-of-band modes (is_valid rejects them, but direct API
        // callers can construct anything) behave as flat, never NaN/garbage.
        let weird = MassingParams {
            roof_mode: 99,
            ..massing_defaults(ZoneType::Downtown)
        };
        let flat_crown = MassingParams {
            roof_mode: 0,
            ..weird
        };
        for lot_id in 0..50 {
            let a = massing_for(lot_id, (7, 7), (3, 3), 60.0, &weird, 5, 4.0);
            let b = massing_for(lot_id, (7, 7), (3, 3), 60.0, &flat_crown, 5, 4.0);
            assert_eq!(a, b);
            assert!(a.is_finite());
        }
    }

    #[test]
    fn degenerate_geometry_never_panics() {
        // Zero rects and stray offsets clamp instead of underflowing (u8
        // arithmetic would wrap in release); outputs stay finite.
        let dt = massing_defaults(ZoneType::Downtown);
        for &(rect, off) in &[
            ((0, 0), (0, 0)),
            ((1, 1), (5, 5)),
            ((7, 7), (200, 200)),
            ((3, 1), (2, 0)),
        ] {
            let h = massing_for(7, rect, off, 60.0, &dt, 69420, 4.0);
            assert!(h.is_finite(), "rect={rect:?} off={off:?} gave {h}");
        }
    }

    #[test]
    fn defaults_cover_all_zones() {
        // Pin the spec table so default drift is caught here, not in viz.
        let (dt, res, com, ind, park) = (
            massing_defaults(ZoneType::Downtown),
            massing_defaults(ZoneType::Residential),
            massing_defaults(ZoneType::Commercial),
            massing_defaults(ZoneType::Industrial),
            massing_defaults(ZoneType::Park),
        );
        assert_eq!((dt.podium_floors, dt.tower_inset, dt.roof_mode), (3, 1, 0));
        assert!((dt.crown_share - 0.3).abs() < 1e-6);
        assert_eq!(
            (res.podium_floors, res.tower_inset, res.roof_mode),
            (0, 0, 1)
        );
        assert_eq!(res.crown_share, 0.0);
        assert_eq!(
            (com.podium_floors, com.tower_inset, com.roof_mode),
            (2, 1, 0)
        );
        assert!((com.crown_share - 0.15).abs() < 1e-6);
        assert_eq!(
            (ind.podium_floors, ind.tower_inset, ind.roof_mode),
            (0, 0, 2)
        );
        assert_eq!(ind.crown_share, 0.0);
        assert_eq!(
            (park.podium_floors, park.tower_inset, park.roof_mode),
            (0, 0, 0)
        );
        assert_eq!(park.crown_share, 0.0);
        // Table order follows ZoneType discriminants.
        let table = default_massing();
        for (i, zone) in ZoneType::all().iter().enumerate() {
            assert_eq!(table[i], massing_defaults(*zone));
        }
    }

    #[test]
    fn helpers_agree_with_steps() {
        // The public helpers examples use recompute exactly what
        // `massing_for` uses internally (no pipeline duplication).
        assert_eq!(massing_ring((7, 7), (0, 3)), 0);
        assert_eq!(massing_ring((7, 7), (3, 3)), 3);
        assert_eq!(massing_ring((1, 1), (0, 0)), 0);
        let dt = massing_defaults(ZoneType::Downtown);
        assert_eq!(podium_top(&dt, 4.0), 12.0);
        // Zero-podium floors still floor at one cell.
        assert_eq!(
            podium_top(&massing_defaults(ZoneType::Residential), 4.0),
            4.0
        );
        // Crown helper matches the stepped outcome on inner cells (flat
        // roof, no jitter inside `massing_for`: crowned ⟺ above H).
        for lot_id in 0..200 {
            let crowned = lot_crowns(lot_id, &dt, 99);
            let h = massing_for(lot_id, (7, 7), (3, 3), 60.0, &dt, 99, 4.0);
            assert_eq!(crowned, h > 60.0, "helper disagrees on lot {lot_id}");
        }
    }
}
