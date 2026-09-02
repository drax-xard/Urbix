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

use crate::hash::hash_coords;
use crate::zones::{ZoneType, ZONE_COUNT};

/// Half-extent of the coordinate span sites are spread across. Sites are
/// placed uniformly in `[-SPAN, SPAN] × [-SPAN, SPAN]`, so a world of roughly
/// 20 000 × 20 000 units is covered while remaining small enough that any
/// point inside the playable area has well-defined nearest sites.
const SPAN: f64 = 10_000.0;

/// Exponent `p` in the Shepard inverse-distance weight `w = 1/d^p`. Higher
/// values sharpen the cells toward a hard Voronoi diagram; lower values
/// soften and round them. `4.0` gives distinct interiors with gentle borders.
const SHEPARD_POWER: f64 = 4.0;

/// Tiny additive constant in the Shepard denominator that prevents
/// division-by-zero when a query lands exactly on a site (or two sites are
/// co-located). Its effect, letting that site's weight dominate the affinity,
/// is exactly what we want at a site's centre.
const SHEPARD_EPSILON: f64 = 1e-8;

/// Domain bytes let the hash primitive separate *uses* of the seed stream,
/// so site x-positions, y-positions, and zone tags never collide spuriously.
const DOMAIN_SITE_X: u8 = 10;
const DOMAIN_SITE_Y: u8 = 11;
const DOMAIN_SITE_ZONE: u8 = 12;

/// Relative frequency of each zone when tagging sites (must sum to 1.0).
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
/// Construct with [`VoronoiDiagram::generate`]; the map is fully determined by
/// `(seed, site_count)` and is intended to live for the whole engine run.
#[derive(Clone, Debug, PartialEq)]
pub struct VoronoiDiagram {
    sites: Vec<VoronoiSite>,
}

impl VoronoiDiagram {
    /// Deterministically generate `site_count` sites spread over the span.
    ///
    /// Every site's position and zone come from the same seed stream, so the
    /// same `(seed, site_count)` always yields the same diagram. `site_count`
    /// should be in the supported 16–64 band (see `config.rs`).
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
        let sites = (0..site_count)
            .map(|i| {
                let idx = i as u64;
                // Independent deterministic samples for x and y.
                let xh = hash_coords(idx as i32, 0, seed, DOMAIN_SITE_X);
                let yh = hash_coords(idx as i32, 0, seed, DOMAIN_SITE_Y);
                let zh = hash_coords(idx as i32, 0, seed, DOMAIN_SITE_ZONE);
                VoronoiSite {
                    x: to_span(xh),
                    y: to_span(yh),
                    zone: pick_zone(zh),
                }
            })
            .collect();
        Self { sites }
    }

    /// Borrow the diagram's sites.
    #[must_use]
    pub fn sites(&self) -> &[VoronoiSite] {
        &self.sites
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
            let w = 1.0 / (d2.powf(SHEPARD_POWER * 0.5) + SHEPARD_EPSILON);
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

/// Map a 53-bit-fraction-encoded hash to a coordinate in `[-SPAN, SPAN]`.
fn to_span(h: u64) -> f64 {
    let t = (h >> 11) as f64 / ((1u64 << 53) as f64);
    -SPAN + t * (2.0 * SPAN)
}

/// Choose a zone by weighted random draw driven by a hash value.
///
/// Walks the cumulative `ZONE_WEIGHTS` distribution; the hash's top bits pick
/// where in `[0, 1)` we land, so the outcome is purely a function of the input.
fn pick_zone(h: u64) -> ZoneType {
    let t = (h >> 11) as f64 / ((1u64 << 53) as f64);
    let mut acc = 0.0f64;
    for zone in ZoneType::all() {
        acc += ZONE_WEIGHTS[zone as usize];
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
