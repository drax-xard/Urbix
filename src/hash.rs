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
//! - Chosen algorithm: a fast, well-distributed hash (e.g. wyhash or SipHash)
//!   — final choice recorded here once Milestone 1 lands.
//!
//! ## Invariant
//!
//! The same `(x, y, seed, domain)` always produces the same `u64`, across
//! calls, runs, and (given a stable algorithm) platforms. This is the backbone
//! of the deterministic, seekable infinite city.

// TODO(Milestone 1): implement hash_coords.
