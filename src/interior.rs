//! # interior.rs
//!
//! Interior hook surface for the Urbix engine.
//!
//! Every built cell is assigned a stable [`crate::data::InteriorId`] during
//! chunk generation (`chunk.rs:interior_id_for`), even though interiors are not
//! yet rendered. This module defines the **hook surface** a future renderer and
//! teleport routine will use, and provides a stub so the interface is wired and
//! callable end-to-end.
//!
//! ## Design
//!
//! - `InteriorState` — trait for a room's data (layout, size, fog, palette).
//!   The trait is intentionally tiny: `fn generate(id, seed) -> Self`.
//! - `PlaceholderInteriorState` — stub that returns deterministic, non-null
//!   placeholder data derived from `hash(id, seed)`. Different seeds produce
//!   different placeholders; the same `(id, seed)` always yields the same state.
//! - `InteriorCache` — bounded cache keyed by `InteriorId`, parallel to
//!   `ChunkCache` but without draw-distance eviction (interiors are a separate
//!   mini-world with their own grid, §4.4). LRU via recency tick.
//!
//! An interior is a *separate mini-world* keyed by `InteriorId`, generated and
//! cached independently of outdoor chunks. Until real room generation lands,
//! `enter`/`exit` are no-ops and `generate` returns the placeholder.

use std::collections::HashMap;

use crate::config::WorldConfig;
use crate::data::InteriorId;
use crate::hash::{domain, hash_coords};

/// Hook trait for a generated interior.
///
/// Future interior types (grid rooms, corridors, etc.) implement this trait.
/// The stub [`PlaceholderInteriorState`] is the minimal implementation that
/// satisfies the Milestone 6 exit criteria.
pub trait InteriorState: Sized + Clone + std::fmt::Debug {
    /// Generate a deterministic interior from a stable `id` and world `seed`.
    ///
    /// The same `(id, seed)` must always yield the same state.
    fn generate(id: InteriorId, seed: u64) -> Self;
}

/// Stub interior state returned before real room generation exists.
///
/// Contains just enough deterministic fields to be useful for tests and to
/// prove the interface is wired: room dimensions, fog density, and palette.
/// All fields are derived from `hash(id, seed, domain)` so they are stable
/// across runs and differ across seeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceholderInteriorState {
    /// The interior key this state was generated from.
    pub id: InteriorId,
    /// World seed used for generation.
    pub seed: u64,
    /// Room width in interior cells (6..14).
    pub width: u16,
    /// Room depth in interior cells (6..14).
    pub height: u16,
    /// Fog density (0..255).
    pub fog: u8,
    /// Palette index for interior walls/floor.
    pub palette_id: u8,
}

impl PlaceholderInteriorState {
    /// Generate with a `WorldConfig` so interior size is tunable via file.
    pub fn generate_with_config(id: InteriorId, seed: u64, config: &WorldConfig) -> Self {
        let x = (id & 0xFFFF_FFFF) as i64;
        let y = ((id >> 32) & 0xFFFF_FFFF) as i64;

        let w_range = config.interior_width_range;
        let h_range = config.interior_height_range;
        let w_span = (w_range[1] - w_range[0] + 1) as u64;
        let h_span = (h_range[1] - h_range[0] + 1) as u64;
        let w_roll = hash_coords(x, y, seed, domain::INTERIOR_SIZE_W);
        let h_roll = hash_coords(x, y, seed, domain::INTERIOR_SIZE_H);
        let width = w_range[0] + (w_roll % w_span) as u16;
        let height = h_range[0] + (h_roll % h_span) as u16;

        let fog = (hash_coords(x, y, seed, domain::INTERIOR_FOG) % 256) as u8;
        let palette_id = (hash_coords(x, y, seed, domain::INTERIOR_PALETTE) % 8) as u8;

        Self {
            id,
            seed,
            width,
            height,
            fog,
            palette_id,
        }
    }
}

impl InteriorState for PlaceholderInteriorState {
    fn generate(id: InteriorId, seed: u64) -> Self {
        Self::generate_with_config(id, seed, &WorldConfig::default())
    }
}

/// Convenience free function mirroring the trait for ergonomic use.
///
/// ```
/// use urbix::interior::{generate_interior, PlaceholderInteriorState};
/// let state = generate_interior::<PlaceholderInteriorState>(42, 445566);
/// assert_eq!(state.id, 42);
/// ```
#[must_use]
pub fn generate_interior<S: InteriorState>(id: InteriorId, seed: u64) -> S {
    S::generate(id, seed)
}

// ---------------------------------------------------------------------------
// InteriorCache — bounded LRU keyed by InteriorId, parallel to ChunkCache
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Entry<S> {
    value: S,
    last_used: u64,
}

/// Bounded cache for generated interiors, keyed by `InteriorId`.
///
/// Unlike `ChunkCache` there is no draw-distance concept — interiors are a
/// separate mini-world (§4.4) — so eviction is purely LRU capacity-based. An
/// interior can always be regenerated deterministically, so dropping it is safe.
///
/// ## Example
///
/// ```
/// use urbix::interior::{InteriorCache, PlaceholderInteriorState};
///
/// let mut cache = InteriorCache::<PlaceholderInteriorState>::new(16);
/// assert!(cache.is_empty());
/// ```
#[derive(Debug)]
pub struct InteriorCache<S> {
    map: HashMap<InteriorId, Entry<S>>,
    capacity: usize,
    tick: u64,
}

impl<S> InteriorCache<S>
where
    S: Clone,
{
    /// Create an empty cache with the given capacity (number of interiors).
    ///
    /// `capacity` of `usize::MAX` means unlimited. A small capacity (e.g. 64)
    /// keeps memory bounded even if many interiors are visited.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            capacity,
            tick: 0,
        }
    }

    /// Insert an interior, updating its recency.
    ///
    /// If the key already existed the old value is replaced. If the cache
    /// exceeds `capacity`, the least-recently-used entries are evicted.
    pub fn insert(&mut self, id: InteriorId, value: S) {
        self.tick += 1;
        self.map.insert(
            id,
            Entry {
                value,
                last_used: self.tick,
            },
        );
        self.evict_if_over_capacity();
    }

    /// Look up a cached interior and touch its recency (LRU).
    #[must_use]
    pub fn get(&mut self, id: &InteriorId) -> Option<&S> {
        if let Some(entry) = self.map.get_mut(id) {
            self.tick += 1;
            entry.last_used = self.tick;
            Some(&entry.value)
        } else {
            None
        }
    }

    /// Number of interiors currently cached.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Drop all cached interiors, retaining capacity configuration.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Update the capacity cap. Triggers eviction if the new cap is smaller.
    pub fn set_capacity(&mut self, cap: usize) {
        self.capacity = cap;
        self.evict_if_over_capacity();
    }

    /// Current capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn evict_if_over_capacity(&mut self) {
        if self.map.len() <= self.capacity {
            return;
        }
        let mut entries: Vec<_> = self.map.iter().map(|(k, e)| (*k, e.last_used)).collect();
        entries.sort_by_key(|(_, t)| *t);
        let excess = self.map.len() - self.capacity;
        for (k, _) in entries.iter().take(excess) {
            self.map.remove(k);
        }
    }
}

impl<S> Default for InteriorCache<S>
where
    S: Clone,
{
    fn default() -> Self {
        Self::new(64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::InteriorId;
    use crate::hash::{domain, hash_coords};

    fn interior_id_for(world_x: i64, world_z: i64, seed: u64) -> InteriorId {
        hash_coords(world_x, world_z, seed, domain::INTERIOR)
    }

    #[test]
    fn interior_id_is_deterministic() {
        let a = interior_id_for(12, -7, 445566);
        let b = interior_id_for(12, -7, 445566);
        assert_eq!(a, b);
        assert_ne!(a, 0);
        // Different coords or seed differ.
        assert_ne!(a, interior_id_for(13, -7, 445566));
        assert_ne!(a, interior_id_for(12, -7, 99));
    }

    #[test]
    fn placeholder_is_deterministic() {
        let a = PlaceholderInteriorState::generate(42, 445566);
        let b = PlaceholderInteriorState::generate(42, 445566);
        assert_eq!(a, b);
        assert_eq!(a.id, 42);
        assert_eq!(a.seed, 445566);
        // Non-null placeholder: dimensions in range, fog/palette populated.
        assert!((6..=14).contains(&a.width));
        assert!((6..=14).contains(&a.height));
    }

    #[test]
    fn different_seeds_produce_different_placeholders() {
        let a = PlaceholderInteriorState::generate(123, 1);
        let b = PlaceholderInteriorState::generate(123, 2);
        assert_ne!(a, b, "different seeds should differ");
        // Also via free function.
        let c: PlaceholderInteriorState = generate_interior(999, 10);
        let d: PlaceholderInteriorState = generate_interior(999, 11);
        assert_ne!(c, d);
    }

    #[test]
    fn interior_cache_basic() {
        let mut cache = InteriorCache::<PlaceholderInteriorState>::new(2);
        let s1 = PlaceholderInteriorState::generate(1, 10);
        let s2 = PlaceholderInteriorState::generate(2, 10);
        let s3 = PlaceholderInteriorState::generate(3, 10);
        cache.insert(1, s1.clone());
        cache.insert(2, s2.clone());
        assert_eq!(cache.len(), 2);
        // Touch s1 so s2 becomes LRU.
        assert!(cache.get(&1).is_some());
        cache.insert(3, s3);
        // Capacity 2 → one eviction, LRU (2) should be gone.
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&2).is_none());
        assert!(cache.get(&1).is_some());
        assert!(cache.get(&3).is_some());
    }

    #[test]
    fn interior_cache_clear() {
        let mut cache = InteriorCache::<PlaceholderInteriorState>::new(8);
        cache.insert(42, PlaceholderInteriorState::generate(42, 1));
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
    }
}
