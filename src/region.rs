//! # region.rs
//!
//! Voronoi region layer for the Urbix city engine.
//!
//! This module defines the *big, smooth* layer of generation: which part of
//! the world is downtown, residential, commercial, industrial, or park. It
//! builds a Voronoi diagram of a fixed set of sites from the seed and answers
//! fuzzy "zone affinity" queries at any world coordinate.
//!
//! ## Fuzzy borders
//!
//! Instead of hard edges, each query blends every site's contribution using a
//! distance-based (Shepard) weighting. The result is a per-point
//! zone-affinity vector that is *continuous everywhere* — even where several
//! cells meet — with the nearest site dominating deep inside its cell and a
//! soft gradient toward its neighbours.
//!
//! ## Longevity
//!
//! The Voronoi map is tiny (24–48 sites) and immutable; it is computed once at
//! engine construction and lives for the whole run. Zone queries therefore
//! stay cheap, and neighbouring chunks remain consistent because they query
//! the same continuous field.

use crate::config::WorldConfig;
use crate::hash::domain;
use crate::hash::{hash_coords, hash_unit};
use crate::lot::{Diagonal, DistrictFrame};
use crate::zones::{ZoneType, ZONE_COUNT};

/// Half-extent of the coordinate span sites are spread across. Sites are
/// placed uniformly in `[-SPAN, SPAN] × [-SPAN, SPAN]`, so a world of roughly
/// 20 000 × 20 000 units is covered while remaining small enough that any
/// point inside the playable area has well-defined nearest sites.
#[allow(dead_code)]
const SPAN: f64 = 10_000.0;

/// Exponent `p` in the Shepard inverse-distance weight `w = 1/d^p`. Higher
/// values sharpen the cells toward a hard Voronoi diagram; lower values
/// soften and round them. `4.0` gives distinct interiors with gentle borders.
#[allow(dead_code)]
const SHEPARD_POWER: f64 = 4.0;

/// Tiny additive constant in the Shepard denominator that prevents
/// division-by-zero when a query lands exactly on a site (or two sites are
/// co-located). Its effect, letting that site's weight dominate the affinity,
/// is exactly what we want at a site's centre.
#[allow(dead_code)]
const SHEPARD_EPSILON: f64 = 1e-8;

/// Relative frequency of each zone when tagging sites (must sum to 1.0).
#[allow(dead_code)]
const ZONE_WEIGHTS: [f64; ZONE_COUNT] = [0.25, 0.30, 0.20, 0.15, 0.10];

/// A single Voronoi site: a point in world space owning one `ZoneType`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoronoiSite {
    /// World-space x position of the site.
    pub x: f64,
    /// World-space y position of the site.
    pub y: f64,
    /// Zone this site belongs to.
    pub zone: ZoneType,
}

/// An immutable Voronoi diagram of district sites derived from a seed.
///
/// Construct with [`VoronoiDiagram::generate`] or [`VoronoiDiagram::generate_with_config`];
/// the map is fully determined by the seed and site configuration and is
/// intended to live for the whole engine run.
///
/// Milestone 11 adds two district-level rules at generation time (zero
/// per-cell cost): the site nearest the origin is forced to Downtown (a
/// legible CBD anchor for the skyline peak), and Industrial sites whose
/// nearest neighbour is Residential are re-tagged Commercial (no grimy
/// factory abutting quiet homes without a buffer).
///
/// Milestone 12 adds two global diagonal boulevards (Broadway-style avenues
/// cutting across district grids, derived from the seed) alongside the sites.
#[derive(Clone, Debug, PartialEq)]
pub struct VoronoiDiagram {
    sites: Vec<VoronoiSite>,
    shepard_power: f64,
    shepard_epsilon: f64,
    seed: u64,
    diagonals: Vec<Diagonal>,
}

impl VoronoiDiagram {
    /// Deterministically generate `site_count` sites spread over the span.
    ///
    /// Every site's position and zone come from the same seed stream, so the
    /// same `(seed, site_count)` always yields the same diagram. `site_count`
    /// should be in the supported 16–64 band (see `config.rs`). This is a
    /// convenience wrapper around [`Self::generate_with_config`] using
    /// `WorldConfig::default()` values for span/power/weights.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::region::VoronoiDiagram;
    ///
    /// let a = VoronoiDiagram::generate(445566, 32);
    /// let b = VoronoiDiagram::generate(445566, 32);
    /// assert_eq!(a.sites().len(), 32);
    /// assert_eq!(a, b); // deterministic
    /// ```
    #[must_use]
    pub fn generate(seed: u64, site_count: u16) -> Self {
        let cfg = WorldConfig {
            seed,
            voronoi_site_count: site_count,
            ..Default::default()
        };
        Self::generate_with_config(&cfg)
    }

    /// Generate sites using a full `WorldConfig` (modular customization,
    /// Milestone 8). Uses `config.voronoi_span`, `shepard_power`,
    /// `shepard_epsilon`, and `zone_weights` instead of the hardcoded defaults.
    #[must_use]
    pub fn generate_with_config(config: &WorldConfig) -> Self {
        let seed = config.seed;
        let site_count = config.voronoi_site_count;
        let span = config.voronoi_span;
        let weights = &config.zone_weights;
        let mut sites: Vec<VoronoiSite> = (0..site_count)
            .map(|i| {
                let idx = i as u64;
                let xh = hash_coords(idx as i64, 0, seed, domain::SITE_X);
                let yh = hash_coords(idx as i64, 0, seed, domain::SITE_Y);
                let zh = hash_coords(idx as i64, 0, seed, domain::SITE_ZONE);
                VoronoiSite {
                    x: to_span_with(xh, span),
                    y: to_span_with(yh, span),
                    zone: pick_zone_with(zh, weights),
                }
            })
            .collect();
        // CBD anchor: the site nearest the origin becomes Downtown so the
        // skyline has one legible peak and `cbd_factor` has a stable centre.
        if let Some(centre) = sites
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.x * a.x + a.y * a.y)
                    .partial_cmp(&(b.x * b.x + b.y * b.y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
        {
            sites[centre].zone = ZoneType::Downtown;
        }
        // Adjacency buffer: an Industrial site whose nearest neighbour is
        // Residential becomes Commercial (order-independent via snapshot).
        if sites.len() > 1 {
            let snapshot: Vec<ZoneType> = sites.iter().map(|s| s.zone).collect();
            for i in 0..sites.len() {
                if snapshot[i] != ZoneType::Industrial {
                    continue;
                }
                let mut best = f64::INFINITY;
                let mut neighbour = ZoneType::Industrial;
                for (j, other) in sites.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let dx = other.x - sites[i].x;
                    let dy = other.y - sites[i].y;
                    let d2 = dx * dx + dy * dy;
                    if d2 < best {
                        best = d2;
                        neighbour = snapshot[j];
                    }
                }
                if neighbour == ZoneType::Residential {
                    sites[i].zone = ZoneType::Commercial;
                }
            }
        }
        // Global diagonal boulevards: two seed-derived avenues spanning the
        // whole map in world coordinates, so they cross chunks and districts
        // seamlessly. The first runs 30–60° off the cardinal grid; the second
        // crosses it at 90° (an X pair, always distinct gestures). Offsets
        // spread across the site span so boulevards land somewhere built.
        let base = hash_unit(0, 0, seed, domain::DIAGONAL);
        let angle0_deg = 30.0 + base * 30.0;
        let sign = if hash_unit(0, 1, seed, domain::DIAGONAL) < 0.5 {
            1.0f64
        } else {
            -1.0f64
        };
        let mut diagonals = Vec::with_capacity(2);
        for (k, extra_deg) in [(0i64, 0.0f64), (1i64, 90.0f64)] {
            let span_choice = hash_unit(k, 2, seed, domain::DIAGONAL) as f64 * 2.0 - 1.0;
            diagonals.push(Diagonal {
                angle_rad: (sign * angle0_deg as f64 + extra_deg) * std::f64::consts::PI / 180.0,
                offset: span_choice * span,
                half_width: 1.0,
            });
        }
        Self {
            sites,
            shepard_power: config.shepard_power,
            shepard_epsilon: config.shepard_epsilon,
            seed,
            diagonals,
        }
    }

    /// Borrow the diagram's sites.
    #[must_use]
    pub fn sites(&self) -> &[VoronoiSite] {
        &self.sites
    }

    /// Borrow the global diagonal boulevards (two seed-derived avenues).
    ///
    /// Defined in absolute world coordinates, so they are seamless across
    /// chunks and districts by construction.
    #[must_use]
    pub fn diagonals(&self) -> &[Diagonal] {
        &self.diagonals
    }

    /// Index of the site nearest `(world_x, world_z)` (linear scan; the map
    /// holds only 16–64 sites so this stays cheap; chunk loops hoist the
    /// frame per distinct district in practice via per-cell memo of one id).
    ///
    /// Ties resolve toward the lower index. Empty diagrams return 0.
    #[must_use]
    pub fn nearest_site_idx(&self, world_x: f64, world_z: f64) -> usize {
        let mut best_idx = 0usize;
        let mut best_d2 = f64::INFINITY;
        for (i, site) in self.sites.iter().enumerate() {
            let dx = site.x - world_x;
            let dy = site.y - world_z;
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
                best_idx = i;
            }
        }
        best_idx
    }

    /// Distance from a world point to the nearest district seam (Voronoi
    /// bisector), in cells.
    ///
    /// Uses the two nearest sites: for the perpendicular bisector the exact
    /// distance is `|d1² − d2²| / (2·gap)`, a pure function of position, so
    /// every chunk agrees on every cell. Returns `INFINITY` for diagrams
    /// with fewer than two sites (no seam can exist).
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::region::VoronoiDiagram;
    /// let d = VoronoiDiagram::generate(42, 32);
    /// // Atop a site we are half a site-gap from any seam: far away.
    /// let s = d.sites()[0];
    /// assert!(d.seam_distance(s.x, s.y) > 1.0);
    /// ```
    #[must_use]
    pub fn seam_distance(&self, world_x: f64, world_z: f64) -> f64 {
        if self.sites.len() < 2 {
            return f64::INFINITY;
        }
        let mut b1 = 0usize;
        let mut b2 = 0usize;
        let mut d1 = f64::INFINITY;
        let mut d2 = f64::INFINITY;
        for (i, site) in self.sites.iter().enumerate() {
            let dx = site.x - world_x;
            let dy = site.y - world_z;
            let d = dx * dx + dy * dy;
            if d < d1 {
                d2 = d1;
                b2 = b1;
                d1 = d;
                b1 = i;
            } else if d < d2 {
                d2 = d;
                b2 = i;
            }
        }
        if !d2.is_finite() {
            return f64::INFINITY;
        }
        let a = &self.sites[b1];
        let b = &self.sites[b2];
        let gap = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
        if gap < 1e-9 {
            return f64::INFINITY;
        }
        (d2 - d1).abs() / (2.0 * gap)
    }

    /// Whether a world cell carries the district-seam parkway.
    ///
    /// Cells within ~1 cell of a bisector become a continuous boundary
    /// boulevard that both adjoining grids tee into — the seam reads as a
    /// parkway instead of a tear. World-space and chunk-consistent like
    /// everything else in this module.
    #[must_use]
    pub fn is_seam_road(&self, world_x: f64, world_z: f64) -> bool {
        self.seam_distance(world_x, world_z) <= 1.0
    }

    /// Per-district street orientation frame at a world coordinate.
    ///
    /// Piecewise constant per nearest site (not blended): every cell whose
    /// nearest site is `i` shares site `i`'s frame, so fabrics differ per
    /// district and meet with intentional jogs at bisectors. The frame
    /// derives from `(site_idx, seed, domain::ORIENTATION)` hashes —
    /// quantized to `{0°, ±10°, ±18°, ±27°}` with a long warp octave
    /// (`0.5–1.5` cells over `40–80`) plus a short wiggle octave (`0.3–1.0`
    /// over `12–30`) — so it is stable across chunks and runs.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::region::VoronoiDiagram;
    /// let d = VoronoiDiagram::generate(42, 32);
    /// let a = d.district_frame_for(100.0, 200.0);
    /// assert_eq!(a, d.district_frame_for(100.0, 200.0));
    /// ```
    #[must_use]
    pub fn district_frame_for(&self, world_x: f64, world_z: f64) -> DistrictFrame {
        if self.sites.is_empty() {
            return DistrictFrame::identity();
        }
        let idx = self.nearest_site_idx(world_x, world_z) as i64;
        let angles = [0.0, 10.0, -10.0, 18.0, -18.0, 27.0, -27.0];
        let pick =
            (hash_coords(idx, 0, self.seed, domain::ORIENTATION) % angles.len() as u64) as usize;
        let angle_rad = angles[pick] * std::f64::consts::PI / 180.0;
        let amp = 0.5 + hash_unit(idx, 1, self.seed, domain::ORIENTATION) as f64;
        let len = 40.0 + hash_unit(idx, 2, self.seed, domain::ORIENTATION) as f64 * 40.0;
        let phase = hash_unit(idx, 3, self.seed, domain::ORIENTATION) as f64
            * std::f64::consts::TAU as f32 as f64;
        let amp2 = 0.3 + hash_unit(idx, 4, self.seed, domain::ORIENTATION) as f64 * 0.7;
        let len2 = 12.0 + hash_unit(idx, 5, self.seed, domain::ORIENTATION) as f64 * 18.0;
        DistrictFrame {
            angle_rad,
            warp_amp: amp,
            warp_len: len,
            warp_phase: phase,
            warp_amp2: amp2,
            warp_len2: len2,
        }
    }

    /// Skyline peak factor at a world coordinate: `1.0` far from downtown,
    /// rising smoothly to `1.5` atop the nearest Downtown site.
    ///
    /// Lorentzian falloff with a 1500-unit knee (`cbd_factor = 1 + 0.5 /
    /// (1 + (d/1500)²)`). Applied to lot heights in `building.rs` via
    /// `chunk.rs` so downtown cores peak and taper instead of plateauing.
    /// Returns `1.0` when no Downtown site exists (unreachable after the CBD
    /// anchor, but safe for hand-built diagrams).
    #[must_use]
    pub fn cbd_factor(&self, world_x: f64, world_z: f64) -> f32 {
        let mut best_d2 = f64::INFINITY;
        for site in &self.sites {
            if site.zone != ZoneType::Downtown {
                continue;
            }
            let dx = site.x - world_x;
            let dy = site.y - world_z;
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
            }
        }
        if !best_d2.is_finite() {
            return 1.0;
        }
        let d = best_d2.sqrt();
        (1.0 + 0.5 / (1.0 + (d / 1500.0) * (d / 1500.0))) as f32
    }

    /// Query the fuzzy zone-affinity vector at an arbitrary world coordinate.
    ///
    /// Returns a length-`ZONE_COUNT` vector (index == [`ZoneType`] variant)
    /// whose entries are non-negative and sum to 1. The nearest site dominates
    /// deep inside its cell; the affinity falls off continuously toward the
    /// border so there are no hard edges or identity snapping — the value
    /// stays continuous even at points where several cells meet.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::region::VoronoiDiagram;
    ///
    /// let d = VoronoiDiagram::generate(42, 32);
    /// let a = d.query(100.0, 200.0);
    /// // Weights are normalised.
    /// assert!((a.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    /// ```
    #[must_use]
    pub fn query(&self, world_x: f64, world_z: f64) -> [f32; ZONE_COUNT] {
        // Shepard's method: weight each site by inverse distance raised to a
        // power, then normalise. Every weight is a continuous function of the
        // position (distance is continuous, and 1/(d^p+eps) is continuous), so
        // the blended affinity is continuous everywhere — no site can "snap"
        // the residual when its rank in the distance ordering changes.
        let mut weighted = [0.0f64; ZONE_COUNT];

        for site in &self.sites {
            let dx = site.x - world_x;
            let dy = site.y - world_z;
            let d2 = dx * dx + dy * dy;
            // The epsilon guard avoids division by zero when the query sits
            // exactly on a site; that term then dominates as expected.
            let w = 1.0 / (d2.powf(self.shepard_power * 0.5) + self.shepard_epsilon);
            weighted[site.zone as usize] += w;
        }

        let total: f64 = weighted.iter().sum();
        // Weights are always positive for a non-empty diagram, so total > 0.
        // The exact-zero guard only exists to keep an empty site list from
        // producing NaNs; we must NOT use a coarse epsilon here, because
        // legitimate queries far from the site cluster yield very small (but
        // positive) totals that still normalise cleanly to a sum of 1.
        if total == 0.0 {
            return [0.0; ZONE_COUNT];
        }
        let inv = 1.0 / total;
        let mut affinity = [0.0f32; ZONE_COUNT];
        for (i, v) in weighted.iter().enumerate() {
            affinity[i] = (v * inv) as f32;
        }
        affinity
    }
}

/// Map a 53-bit-fraction-encoded hash to a coordinate in `[-span, span]`.
#[allow(dead_code)]
fn to_span(h: u64) -> f64 {
    to_span_with(h, SPAN)
}

fn to_span_with(h: u64, span: f64) -> f64 {
    let t = (h >> 11) as f64 / ((1u64 << 53) as f64);
    -span + t * (2.0 * span)
}

/// Choose a zone by weighted random draw driven by a hash value.
///
/// Walks the cumulative `ZONE_WEIGHTS` distribution; the hash's top bits pick
/// where in `[0, 1)` we land, so the outcome is purely a function of the input.
#[allow(dead_code)]
fn pick_zone(h: u64) -> ZoneType {
    pick_zone_with(h, &ZONE_WEIGHTS)
}

fn pick_zone_with(h: u64, weights: &[f64; ZONE_COUNT]) -> ZoneType {
    let t = (h >> 11) as f64 / ((1u64 << 53) as f64);
    let mut acc = 0.0f64;
    for zone in ZoneType::all() {
        acc += weights[zone as usize];
        if t < acc {
            return zone;
        }
    }
    // Floating-point tail: t landed (numerically) at the very top; fall back
    // to the last zone rather than leaving it unassigned.
    *ZoneType::all().last().expect("ZONE_COUNT > 0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(
            VoronoiDiagram::generate(7, 32),
            VoronoiDiagram::generate(7, 32)
        );
        assert_eq!(
            VoronoiDiagram::generate(u64::MAX, 24),
            VoronoiDiagram::generate(u64::MAX, 24)
        );
    }

    #[test]
    fn generation_differs_across_seeds() {
        assert_ne!(
            VoronoiDiagram::generate(1, 32),
            VoronoiDiagram::generate(2, 32)
        );
        assert_ne!(
            VoronoiDiagram::generate(7, 32),
            VoronoiDiagram::generate(8, 24)
        );
    }

    #[test]
    fn site_count_is_respected() {
        assert_eq!(VoronoiDiagram::generate(5, 24).sites().len(), 24);
        assert_eq!(VoronoiDiagram::generate(5, 48).sites().len(), 48);
    }

    #[test]
    fn sites_lie_within_span() {
        for site in VoronoiDiagram::generate(12345, 48).sites() {
            assert!(site.x.abs() <= SPAN + 1e-6);
            assert!(site.y.abs() <= SPAN + 1e-6);
        }
    }

    #[test]
    fn query_is_deterministic() {
        let d = VoronoiDiagram::generate(99, 32);
        let a = d.query(123.0, -456.0);
        let b = d.query(123.0, -456.0);
        assert_eq!(a, b);
    }

    #[test]
    fn cbd_anchor_forces_downtown_near_origin() {
        // Every seed gets a legible CBD: the site nearest the origin is Downtown.
        for seed in [1u64, 7, 42, 445566] {
            let d = VoronoiDiagram::generate(seed, 32);
            let mut best_idx = 0usize;
            let mut best_d2 = f64::INFINITY;
            for (i, s) in d.sites().iter().enumerate() {
                let d2 = s.x * s.x + s.y * s.y;
                if d2 < best_d2 {
                    best_d2 = d2;
                    best_idx = i;
                }
            }
            assert_eq!(d.sites()[best_idx].zone, ZoneType::Downtown);
        }
    }

    #[test]
    fn industrial_never_abuts_residential() {
        // Adjacency buffer: no Industrial site's nearest neighbour is Residential.
        for seed in [1u64, 7, 42, 99] {
            let d = VoronoiDiagram::generate(seed, 48);
            let sites = d.sites();
            for (i, s) in sites.iter().enumerate() {
                if s.zone != ZoneType::Industrial {
                    continue;
                }
                let mut best = f64::INFINITY;
                let mut neighbour = ZoneType::Industrial;
                for (j, o) in sites.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let d2 = (o.x - s.x).powi(2) + (o.y - s.y).powi(2);
                    if d2 < best {
                        best = d2;
                        neighbour = o.zone;
                    }
                }
                assert_ne!(neighbour, ZoneType::Residential, "seed {seed} site {i}");
            }
        }
    }

    #[test]
    fn district_frame_is_deterministic_and_quantized() {
        let d = VoronoiDiagram::generate(42, 32);
        let a = d.district_frame_for(100.0, 200.0);
        assert_eq!(a, d.district_frame_for(100.0, 200.0));
        // Angle is one of the quantized set; warps stay in band.
        let deg = a.angle_rad * 180.0 / std::f64::consts::PI;
        assert!(
            [0.0, 10.0, -10.0, 18.0, -18.0, 27.0, -27.0]
                .iter()
                .any(|v| (v - deg).abs() < 1e-9),
            "angle {deg} not quantized"
        );
        assert!((0.5..=1.5).contains(&a.warp_amp));
        assert!((40.0..=80.0).contains(&a.warp_len));
        assert!((0.3..=1.0).contains(&a.warp_amp2));
        assert!((12.0..=30.0).contains(&a.warp_len2));
    }

    #[test]
    fn diagonals_are_deterministic_distinct_and_global() {
        // Two boulevards per diagram, stable across calls, 90° apart.
        let d = VoronoiDiagram::generate(42, 32);
        let ds = d.diagonals();
        assert_eq!(ds.len(), 2);
        assert_eq!(ds, VoronoiDiagram::generate(42, 32).diagonals());
        let sep =
            ((ds[0].angle_rad - ds[1].angle_rad).abs() * 180.0 / std::f64::consts::PI) % 180.0;
        assert!((sep - 90.0).abs() < 1e-6, "separation {sep}");
        // Global: defined in world coords, so a boulevard crosses the map —
        // sweeping perpendicular finds pavement far from the origin.
        for diag in ds {
            let mut covered = 0;
            for t in (-5000..5000).step_by(7) {
                // Walk the normal direction; count cells on the pavement.
                let nx = -diag.angle_rad.sin();
                let nz = diag.angle_rad.cos();
                let wx = (diag.offset * nx + t as f64 * diag.angle_rad.cos()) as i64;
                let wz = (diag.offset * nz + t as f64 * diag.angle_rad.sin()) as i64;
                if diag.covers(wx, wz) {
                    covered += 1;
                }
            }
            assert!(covered > 100, "boulevard covers nothing");
        }
    }

    #[test]
    fn seam_road_hugs_bisectors_not_site_cores() {
        // Midpoints between nearby sites sit on the seam; site cores are far.
        // Pairs whose border never reaches the midpoint (a third site owns
        // it) are skipped — the assertion only covers true a/b borders.
        let d = VoronoiDiagram::generate(42, 32);
        let sites = d.sites();
        let mut found = 0;
        for (i, a) in sites.iter().enumerate() {
            // Nearest neighbour of `a`.
            let mut best = f64::INFINITY;
            let mut nb = 0usize;
            for (j, b) in sites.iter().enumerate() {
                if i == j {
                    continue;
                }
                let d2 = (a.x - b.x).powi(2) + (a.y - b.y).powi(2);
                if d2 < best {
                    best = d2;
                    nb = j;
                }
            }
            if best.sqrt() > 3000.0 {
                continue; // only close pairs give crisp seams
            }
            let b = &sites[nb];
            let mx = (a.x + b.x) / 2.0;
            let mz = (a.y + b.y) / 2.0;
            let mid_to_a = (best / 4.0).sqrt();
            let owned_by_third = sites.iter().enumerate().any(|(k, s)| {
                k != i
                    && k != nb
                    && ((s.x - mx).powi(2) + (s.y - mz).powi(2)).sqrt() < mid_to_a - 1e-9
            });
            if owned_by_third {
                continue;
            }
            assert!(
                d.is_seam_road(mx, mz),
                "midpoint of close pair is not a seam road"
            );
            found += 1;
        }
        assert!(found > 0, "no close site pairs sampled");
        // Cores stay road-free.
        for s in sites {
            assert!(!d.is_seam_road(s.x, s.y), "site core flagged as seam");
        }
    }

    #[test]
    fn seam_queries_are_deterministic() {
        let d = VoronoiDiagram::generate(7, 32);
        assert_eq!(
            d.seam_distance(123.0, -456.0),
            d.seam_distance(123.0, -456.0)
        );
        assert_eq!(d.is_seam_road(123.0, -456.0), d.is_seam_road(123.0, -456.0));
    }

    #[test]
    fn cbd_factor_peaks_downtown_and_fades() {
        let d = VoronoiDiagram::generate(7, 32);
        let downtown = d
            .sites()
            .iter()
            .find(|s| s.zone == ZoneType::Downtown)
            .copied()
            .expect("CBD anchor guarantees a downtown site");
        let peak = d.cbd_factor(downtown.x, downtown.y);
        assert!((peak - 1.5).abs() < 1e-6, "peak {peak}");
        let far = d.cbd_factor(downtown.x + 20_000.0, downtown.y);
        assert!((1.0..1.05).contains(&far), "far {far}");
    }

    #[test]
    fn affinity_at_site_is_near_one_for_its_zone() {
        for seed in [1u64, 7, 42] {
            let d = VoronoiDiagram::generate(seed, 32);
            for site in d.sites() {
                let aff = d.query(site.x, site.y);
                // Nearest site dominates at its own position.
                assert!(
                    aff[site.zone as usize] > 0.99,
                    "seed {seed} zone {:?}",
                    site.zone
                );
            }
        }
    }

    #[test]
    fn affinity_sums_to_one() {
        let d = VoronoiDiagram::generate(445566, 32);
        for xi in -50..50 {
            for yi in -50..50 {
                let aff = d.query(xi as f64 * 400.0, yi as f64 * 400.0);
                let sum: f32 = aff.iter().sum();
                assert!(
                    (sum - 1.0).abs() < 1e-5,
                    "affinity does not sum to 1 at ({xi},{yi}): {aff:?}"
                );
            }
        }
    }

    #[test]
    fn query_is_continuous() {
        // A tiny step in world space must not shift the affinity much.
        let d = VoronoiDiagram::generate(314159, 32);
        let step = 0.01;
        for xi in -20..20 {
            for yi in -20..20 {
                let wx = xi as f64 * 500.0;
                let wz = yi as f64 * 500.0;
                let a = d.query(wx, wz);
                let b = d.query(wx + step, wz + step);
                for k in 0..ZONE_COUNT {
                    assert!(
                        (a[k] - b[k]).abs() < 0.001,
                        "large affinity jump at ({wx},{wz}), zone {k}: {a:?} vs {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn query_is_continuous_across_site_bisectors() {
        // The stress case for continuity: sweeping along the bisector of
        // every nearby site pair, where the ranking of sites (and hence the
        // blend) must reorder without any discontinuous snap.
        for seed in [1u64, 7, 42, 99, 314159] {
            let d = VoronoiDiagram::generate(seed, 48);
            let sites = d.sites().to_vec();
            let step = 0.001;
            for i in 0..sites.len() {
                for j in 0..sites.len() {
                    if i == j {
                        continue;
                    }
                    let a = &sites[i];
                    let b = &sites[j];
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    // Only inspect reasonably close pairs (the interesting
                    // borderline geometry); distant pairs stay flat.
                    if dist > 3000.0 {
                        continue;
                    }
                    let mx = (a.x + b.x) / 2.0;
                    let my = (a.y + b.y) / 2.0;
                    for k in 0..200 {
                        let t = (k as f64) / 200.0 - 0.5;
                        let px = mx + t * dx * 0.5;
                        let py = my + t * dy * 0.5;
                        let q1 = d.query(px, py);
                        let q2 = d.query(px + step, py + step);
                        for z in 0..ZONE_COUNT {
                            assert!(
                                (q1[z] - q2[z]).abs() < 0.001,
                                "jump seed {seed} zone {z} at ({px},{py}): {q1:?} -> {q2:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
