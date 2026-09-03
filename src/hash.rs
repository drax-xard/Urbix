//! # hash.rs
//!
//! Deterministic seeded hashing for the Urbix engine.
//!
//! Every piece of generated content — a chunk, a building height, a facade
//! palette, an interior — is derived from a small, deterministic hash of its
//! coordinates and a domain byte. This is the single primitive that makes the
//! whole world reproducible from a seed without any persistent global RNG.
//!
//! ## Design
//!
//! - `hash_coords(x, y, seed, domain) -> u64`
//! - The `domain` byte separates *uses* of a hash, so, e.g., a building's
//!   height and its palette produce different values even for the same cell.
//! - Chosen algorithm: a self-contained SplitMix64-inspired finalizer over a
//!   seed-keyed accumulator. It is a fast, well-distributed 64-bit mix with no
//!   branches or table lookups, so it is constant-time, stable across
//!   platforms, and cheap enough for millions of per-cell calls.
//!
//! ## Invariant
//!
//! The same `(x, y, seed, domain)` always produces the same `u64`, across
//! calls, runs, and (given a stable algorithm) platforms. This is the backbone
//! of the deterministic, seekable infinite city.

// SplitMix64-style mixing constant.
const GOLDEN_RATIO: u64 = 0x9e37_79b9_7f4a_7c15;

/// Domain byte separating generation *uses* of the hash stream.
///
/// Each distinct kind of derived value gets its own domain so that hashing the
/// same coordinates for different purposes (building height, palette, interior
/// id, Voronoi site placement, ...) never collides spuriously. Modules should
/// follow the reserved bands below when introducing a new use.
pub mod domain {
    /// Voronoi site x-positions (see `region.rs`).
    pub const SITE_X: u8 = 10;
    /// Voronoi site y-positions (see `region.rs`).
    pub const SITE_Y: u8 = 11;
    /// Voronoi site zone tags (see `region.rs`).
    pub const SITE_ZONE: u8 = 12;

    /// Building height (see `building.rs`).
    pub const HEIGHT: u8 = 20;
    /// Building facade palette id (see `building.rs`).
    pub const PALETTE: u8 = 21;
    /// Building footprint sparsity / empty-lot roll (see `building.rs`).
    pub const DENSITY: u8 = 22;
    /// Interior id for a built cell (see `chunk.rs`).
    pub const INTERIOR: u8 = 23;
}

/// Hash a coordinate pair under a seed and domain into a `u64`.
///
/// The four inputs are folded into a single accumulator: `x` and `y` are the
/// spatial coordinates of the query point, `seed` is the world seed, and
/// `domain` is an extra byte separating distinct *uses* of the hash (heights,
/// palettes, interior ids, ...). Changing any one of them scrambles the output.
///
/// ## Coordinates are 64-bit
///
/// `x`/`y` are `i64` because the city is infinite: a 32-bit cell coordinate
/// would overflow once a player wanders beyond ~2^26 chunks, corrupting the
/// hash (and crashing in debug builds). Using `i64` keeps generation correct
/// across the entire reachable world. For an `i32`-range value, the output is
/// byte-for-byte identical to the old `i32` signature (both sign-extend the
/// value into `u64`), so widening breaks no existing determinism.
///
/// ## Determinism
///
/// This function is pure and deterministic: the same arguments always yield
/// the same value, on any run and any platform. There is no global RNG state.
///
/// ## Example
///
/// ```
/// use urbix::hash::hash_coords;
///
/// // Same inputs => same output.
/// assert_eq!(hash_coords(3, 7, 445566, 1), hash_coords(3, 7, 445566, 1));
/// // Different seed or domain => different output.
/// assert_ne!(hash_coords(3, 7, 1, 1), hash_coords(3, 7, 2, 1));
/// assert_ne!(hash_coords(3, 7, 445566, 1), hash_coords(3, 7, 445566, 2));
/// ```
#[must_use]
pub fn hash_coords(x: i64, y: i64, seed: u64, domain: u8) -> u64 {
    // Fold every input into a single 64-bit accumulator. Each field is
    // multiplied by a distinct odd constant so that small changes (a single
    // coordinate step, a different domain) spread across the whole value.
    // The coordinate cast to u64 preserves sign so negative chunks (west/south
    // of origin) hash differently from positive ones.
    let mut acc = seed
        ^ (domain as u64).wrapping_mul(0x6a09_e667_f3bc_c909)
        ^ (x as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ (y as u64).wrapping_mul(0x94d0_49bb_1331_11eb);

    // SplitMix64 finalizer: a sequence of xor-shift and multiply steps
    // avalanche the state so the output's low bits depend on all input bits.
    acc = acc.wrapping_add(GOLDEN_RATIO);
    acc = (acc ^ (acc >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    acc = (acc ^ (acc >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    acc ^ (acc >> 31)
}

/// Hash a coordinate pair into a `[0, 1)` `f32` for ratio/lottery draws.
///
/// A thin wrapper over [`hash_coords`] that normalises the 64-bit output into
/// the unit interval. Used where generations need a fraction — building height
/// interpolation, density rolls — rather than a raw key. The `domain` byte
/// keeps each such draw independent from the others.
///
/// ## Example
///
/// ```
/// use urbix::hash::hash_unit;
///
/// // In the unit interval, and deterministic.
/// let t = hash_unit(3, 7, 445566, 1);
/// assert!((0.0..1.0).contains(&t));
/// assert_eq!(t, hash_unit(3, 7, 445566, 1));
/// ```
#[must_use]
pub fn hash_unit(x: i64, y: i64, seed: u64, domain: u8) -> f32 {
    // Take the high 24 bits of the mix to get a decently-spread f32 in [0,1).
    // Dividing by 2^24 keeps the result strictly below 1.0.
    (hash_coords(x, y, seed, domain) >> 40) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::hash_coords;

    #[test]
    fn deterministic_across_calls() {
        assert_eq!(hash_coords(3, 7, 445566, 1), hash_coords(3, 7, 445566, 1));
        assert_eq!(hash_coords(-5, 12, 99, 0), hash_coords(-5, 12, 99, 0));
        assert_eq!(
            hash_coords(i64::MAX, i64::MIN, u64::MAX, 255),
            hash_coords(i64::MAX, i64::MIN, u64::MAX, 255)
        );
    }

    #[test]
    fn differs_across_domains_for_same_coords() {
        let a = hash_coords(3, 7, 445566, 1);
        let b = hash_coords(3, 7, 445566, 2);
        let c = hash_coords(3, 7, 445566, 3);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn differs_across_seeds() {
        assert_ne!(hash_coords(3, 7, 1, 1), hash_coords(3, 7, 2, 1));
        assert_ne!(hash_coords(3, 7, 0, 0), hash_coords(3, 7, 12345, 0));
    }

    #[test]
    fn differs_across_coordinates() {
        assert_ne!(hash_coords(0, 0, 445566, 1), hash_coords(1, 0, 445566, 1));
        assert_ne!(hash_coords(0, 0, 445566, 1), hash_coords(0, 1, 445566, 1));
        // Negative coordinates must not collide with their magnitudes.
        assert_ne!(hash_coords(-1, 2, 7, 1), hash_coords(1, 2, 7, 1));
    }

    #[test]
    fn outputs_are_nonzero_for_typical_inputs() {
        // Guards against accidentally falling into a fixed point (e.g. all
        // inputs zero should still avalanche, though colliding zero is fine
        // as a theoretical possibility, we just check a few common cases).
        assert_ne!(hash_coords(0, 0, 0, 0), 0);
        assert_ne!(hash_coords(1, 1, 1, 1), 0);
    }

    #[test]
    fn i32_range_values_match_legacy_sign_extension() {
        // Widening from i32 to i64 must not change the result for values that
        // fit in i32: the legacy path folded `x as i64 as u64` (sign-extend),
        // which is byte-identical to folding `x as u64` on the i64 arg. Rebuild
        // the legacy i32 formula here and require an exact match as a
        // regression guard for the documented backward-compat guarantee.
        fn legacy32(x: i32, y: i32, s: u64, d: u8) -> u64 {
            let c1 = 0x6a09_e667_f3bc_c909u64;
            let c2 = 0xbf58_476d_1ce4_e5b9u64;
            let c3 = 0x94d0_49bb_1331_11ebu64;
            let g = 0x9e37_79b9_7f4a_7c15u64;
            let mut acc = s
                ^ (d as u64).wrapping_mul(c1)
                ^ (x as i64 as u64).wrapping_mul(c2)
                ^ (y as i64 as u64).wrapping_mul(c3);
            acc = acc.wrapping_add(g);
            acc = (acc ^ (acc >> 30)).wrapping_mul(c2);
            acc = (acc ^ (acc >> 27)).wrapping_mul(c3);
            acc ^ (acc >> 31)
        }
        for (x, y, s, d) in [
            (0, 0, 0, 0),
            (3, -7, 445566, 1),
            (-45, 200, 99, 5),
            (i32::MIN, i32::MAX, u64::MAX, 9),
        ] {
            assert_eq!(hash_coords(x as i64, y as i64, s, d), legacy32(x, y, s, d));
        }
    }

    #[test]
    fn wide_coordinates_do_not_collapse() {
        // Very large coordinates (beyond the old i32 range) still produce
        // varied, deterministic output rather than wrapping into collisions.
        let a = hash_coords(1i64 << 40, 0, 7, 2);
        let b = hash_coords((1i64 << 40) + 1, 0, 7, 2);
        assert_ne!(a, b);
        assert_ne!(
            hash_coords(i64::MIN, 0, 7, 2),
            hash_coords(i64::MAX, 0, 7, 2)
        );
    }
}
