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

/// Hash a coordinate pair under a seed and domain into a `u64`.
///
/// The four inputs are folded into a single accumulator: `x` and `y` are the
/// spatial coordinates of the query point, `seed` is the world seed, and
/// `domain` is an extra byte separating distinct *uses* of the hash (heights,
/// palettes, interior ids, ...). Changing any one of them scrambles the output.
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
pub fn hash_coords(x: i32, y: i32, seed: u64, domain: u8) -> u64 {
    // Fold every input into a single 64-bit accumulator. Each field is
    // multiplied by a distinct odd constant so that small changes (a single
    // coordinate step, a different domain) spread across the whole value.
    // The coordinate casts to i64-as-u64 preserve sign so negative chunks
    // (west/south of origin) hash differently from positive ones.
    let mut acc = seed
        ^ (domain as u64).wrapping_mul(0x6a09_e667_f3bc_c909)
        ^ (x as i64 as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ (y as i64 as u64).wrapping_mul(0x94d0_49bb_1331_11eb);

    // SplitMix64 finalizer: a sequence of xor-shift and multiply steps
    // avalanche the state so the output's low bits depend on all input bits.
    acc = acc.wrapping_add(GOLDEN_RATIO);
    acc = (acc ^ (acc >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    acc = (acc ^ (acc >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    acc ^ (acc >> 31)
}

#[cfg(test)]
mod tests {
    use super::hash_coords;

    #[test]
    fn deterministic_across_calls() {
        assert_eq!(hash_coords(3, 7, 445566, 1), hash_coords(3, 7, 445566, 1));
        assert_eq!(hash_coords(-5, 12, 99, 0), hash_coords(-5, 12, 99, 0));
        assert_eq!(
            hash_coords(i32::MAX, i32::MIN, u64::MAX, 255),
            hash_coords(i32::MAX, i32::MIN, u64::MAX, 255)
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
}
