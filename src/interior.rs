//! # interior.rs
//!
//! Interior generation surface for the Urbix engine.
//!
//! Every built cell is assigned a stable [`crate::data::InteriorId`] during
//! chunk generation (`chunk.rs:interior_id_for`). This module defines the
//! **generation surface** a renderer and teleport routine will use: the
//! [`InteriorState`] trait parameterized by the exterior lot's context
//! ([`crate::layout::InteriorContext`]).
//!
//! ## Design
//!
//! - `InteriorState` — trait for a generated interior: `fn generate(id, ctx) -> Self`.
//!   The trait is intentionally tiny; the *context* (zone, floors, footprint)
//!   carries all exterior information the generator needs, so a skyscraper and
//!   a home produce distinct interiors without the trait growing.
//! - `PlaceholderInteriorState` — stub returning deterministic placeholder data
//!   sized from the context's footprint, so the interface is wired end-to-end.
//! - `InteriorCache` — bounded cache keyed by `InteriorId`, parallel to
//!   `ChunkCache` but without draw-distance eviction (interiors are a separate
//!   mini-world with their own grid, §4.4). LRU via recency tick.
//!
//! An interior is a *separate mini-world* keyed by `InteriorId`, generated and
//! cached independently of outdoor chunks. Full room layout (rooms, corridors,
//! doors, furniture) is driven by the per-zone [`crate::layout::Blueprint`]
//! tables; the baseline generator in this module produces a deterministic,
//! walled grid from the context and blueprint so the surface is exercisable
//! end-to-end before the richer algorithm lands.

use std::collections::HashMap;

use crate::config::WorldConfig;
use crate::data::InteriorId;
use crate::hash::{domain, hash_coords};
use crate::layout::{Blueprint, Floor, InteriorContext, InteriorLayout, Tile};

/// Hook trait for a generated interior.
///
/// Implementors generate a deterministic interior from a stable `id` and the
/// exterior lot's [`InteriorContext`]. The same `(id, ctx)` must always yield
/// the same state; `ctx` is derived deterministically from the cell, so
/// interior and exterior stay consistent. [`PlaceholderInteriorState`] is the
/// minimal implementation that satisfies the Milestone 6/9 exit criteria.
pub trait InteriorState: Sized + Clone + std::fmt::Debug {
    /// Generate a deterministic interior from a stable `id` and exterior
    /// `ctx`.
    ///
    /// The same `(id, ctx)` must always yield the same state.
    fn generate(id: InteriorId, ctx: &InteriorContext) -> Self;
}

/// Stub interior state returned before full room generation lands.
///
/// Contains just enough deterministic fields to be useful for tests and to
/// prove the interface is wired: room dimensions derived from the context's
/// footprint, fog density, and palette. All fields are derived from
/// `hash(id, seed, domain)` (clamped by `ctx`) so they are stable across runs
/// and differ across seeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceholderInteriorState {
    /// The interior key this state was generated from.
    pub id: InteriorId,
    /// World seed used for generation.
    pub seed: u64,
    /// Interior grid width in tiles (from context footprint).
    pub width: u16,
    /// Interior grid depth in tiles (from context footprint).
    pub height: u16,
    /// Number of floors (from context height).
    pub floors: u8,
    /// Fog density (0..255).
    pub fog: u8,
    /// Palette index for interior walls/floor.
    pub palette_id: u8,
}

impl PlaceholderInteriorState {
    /// Generate with a `WorldConfig` so interior size is tunable via file.
    ///
    /// Grid dimensions come from the context's footprint (the exterior block),
    /// clamped into the config's interior size ranges so degenerate footprints
    /// stay bounded and valid.
    pub fn generate_with_config(
        id: InteriorId,
        ctx: &InteriorContext,
        config: &WorldConfig,
    ) -> Self {
        let x = (id & 0xFFFF_FFFF) as i64;
        let y = ((id >> 32) & 0xFFFF_FFFF) as i64;

        let w_range = config.interior_width_range;
        let h_range = config.interior_height_range;
        let w_span = (w_range[1] - w_range[0] + 1) as u64;
        let h_span = (h_range[1] - h_range[0] + 1) as u64;
        let w_roll = hash_coords(x, y, ctx.seed, domain::INTERIOR_SIZE_W);
        let h_roll = hash_coords(x, y, ctx.seed, domain::INTERIOR_SIZE_H);

        // Prefer the exterior footprint; only fall back to the config range
        // when the footprint is unset (degenerate lot).
        let width = if ctx.footprint_w > 0 {
            u16::from(ctx.footprint_w)
        } else {
            w_range[0] + (w_roll % w_span) as u16
        };
        let height = if ctx.footprint_d > 0 {
            u16::from(ctx.footprint_d)
        } else {
            h_range[0] + (h_roll % h_span) as u16
        };

        let fog = (hash_coords(x, y, ctx.seed, domain::INTERIOR_FOG) % 256) as u8;
        let palette_id = if ctx.palette_id != 0 {
            ctx.palette_id
        } else {
            (hash_coords(x, y, ctx.seed, domain::INTERIOR_PALETTE) % 8) as u8
        };

        Self {
            id,
            seed: ctx.seed,
            width,
            height,
            floors: ctx.floor_count,
            fog,
            palette_id,
        }
    }
}

impl InteriorState for PlaceholderInteriorState {
    fn generate(id: InteriorId, ctx: &InteriorContext) -> Self {
        Self::generate_with_config(id, ctx, &WorldConfig::default())
    }
}

/// Convenience free function mirroring the trait for ergonomic use.
///
/// ```
/// use urbix::interior::{generate_interior, PlaceholderInteriorState};
/// use urbix::layout::{InteriorContext, blueprint_defaults};
/// use urbix::zones::ZoneType;
///
/// let ctx = InteriorContext {
///     id: 42,
///     zone: ZoneType::Residential,
///     zone_affinity: [0.0; 5],
///     height: 10.0,
///     floor_count: 2,
///     footprint_w: 8,
///     footprint_d: 8,
///     palette_id: 1,
///     seed: 445566,
/// };
/// let state = generate_interior::<PlaceholderInteriorState>(42, &ctx);
/// assert_eq!(state.id, 42);
/// ```
#[must_use]
pub fn generate_interior<S: InteriorState>(id: InteriorId, ctx: &InteriorContext) -> S {
    S::generate(id, ctx)
}

/// Generate a deterministic, walled [`InteriorLayout`] baseline from an
/// exterior context and a zone blueprint (Milestone 9).
///
/// This is the *data-model* generator: it turns a lot's context (zone, floors,
/// footprint) plus its blueprint into one [`Floor`] grid per storey. Each floor
/// is carved as a ring of exterior `Wall` tiles with a concrete `Corridor` and
/// `Core` (stairs/elevator) painted from the context hash — everything placed
/// deterministically. It deliberately produces a *conservative, sealed* result
/// (no tile leaks out of the footprint) that the richer room-placement
/// algorithm and renderers build on.
///
/// The full blueprint-driven room carving (rooms/doors/furniture from the
/// weighted table) is a follow-on step; this establishes the stable output shape
/// and exercises the context plumbing end-to-end.
#[must_use]
pub fn generate_layout(
    id: InteriorId,
    ctx: &InteriorContext,
    blueprint: &Blueprint,
) -> InteriorLayout {
    let seed = ctx.seed;
    let (x_id, y_id) = split_id(id);

    // Number of floors defaults to the context; a degenerate footprint still
    // yields a usable single floor so the result is never empty.
    let floor_count = ctx.floor_count.max(1);
    let floors = (0..floor_count)
        .map(|_floor| {
            let mut g = Floor::empty(ctx.footprint_w, ctx.footprint_d);

            // Outer wall ring: seal the footprint so nothing leaks outside.
            if g.width >= 3 && g.depth >= 3 {
                let w = g.width - 1;
                let d = g.depth - 1;
                for x in 0..=w {
                    let i0 = g.index(x, 0);
                    let id_ = g.index(x, d);
                    g.tiles[i0] = Tile::Wall;
                    g.tiles[id_] = Tile::Wall;
                }
                for z in 0..=d {
                    let i0 = g.index(0, z);
                    let iw = g.index(w, z);
                    g.tiles[i0] = Tile::Wall;
                    g.tiles[iw] = Tile::Wall;
                }
            } else {
                // Degenerate footprint: mark the whole grid solid so the
                // interior never exposes an unwalled void.
                g.tiles.fill(Tile::Wall);
            }

            // Vertical circulation core: a filled square near the centre whose
            // position is hashed from the id + floor. Clamp inside the wall ring.
            let core = blueprint.core_size.max(1);
            let inner_span_w = g.width.saturating_sub(core + 1).max(1);
            let inner_span_d = g.depth.saturating_sub(core + 1).max(1);
            let cx_lo = hash_coords(x_id, y_id, seed, domain::LAYOUT_FLOOR) as usize;
            let cx = 1u8 + (cx_lo % usize::from(inner_span_w)) as u8;
            let cz = 1u8 + ((cx_lo >> 16) % usize::from(inner_span_d)) as u8;
            paint_core(&mut g, cx, cz, core);

            // One concrete corridor from the core to the west edge (door at the
            // boundary) so there is a guaranteed navigable path.
            if g.width >= 3 && g.depth >= 3 {
                let mut z = cz;
                let z_end = cz + core.saturating_sub(1);
                while z <= z_end {
                    let idoor = g.index(0, z);
                    let icorr = g.index(1, z);
                    g.tiles[idoor] = Tile::Door;
                    g.tiles[icorr] = Tile::Corridor;
                    z += 1;
                }
                let cx_clamped = cx.min(g.width - 2);
                let i_cx = g.index(cx_clamped, cz);
                g.tiles[i_cx] = Tile::Corridor;
            }

            // Record the room-kind tag where the floor is a corridor/room tile,
            // using the first blueprint room as the default interior tag.
            let default_kind = blueprint.room_slice().first().map_or(0, |r| r.kind);
            for (i, t) in g.tiles.iter().enumerate() {
                if matches!(t, Tile::Corridor | Tile::Room | Tile::Door) {
                    g.kinds[i] = default_kind;
                }
            }

            g
        })
        .collect::<Vec<_>>();

    InteriorLayout {
        id,
        seed,
        context: *ctx,
        floors,
    }
}

/// Paint a filled `core×core` square of `Tile::Core` tiles centred near
/// `(cx0, cz0)`, clamped inside the floor grid, skipping the wall ring.
fn paint_core(g: &mut Floor, cx0: u8, cz0: u8, core: u8) {
    let w = usize::from(g.width);
    let d = usize::from(g.depth);
    for dz in 0..core {
        for dx in 0..core {
            let x = usize::from(cx0) + usize::from(dx);
            let z = usize::from(cz0) + usize::from(dz);
            if x >= 1 && z >= 1 && x + 1 < w && z + 1 < d {
                let idx = z * w + x;
                g.tiles[idx] = Tile::Core;
                g.kinds[idx] = 0;
            }
        }
    }
}

/// Split an `InteriorId` into its coordinate halves for hashing (matches the
/// placeholder's split and the id's origin as a cell-coordinate hash).
fn split_id(id: InteriorId) -> (i64, i64) {
    ((id & 0xFFFF_FFFF) as i64, ((id >> 32) & 0xFFFF_FFFF) as i64)
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
    use crate::layout::{blueprint_defaults, InteriorContext};
    use crate::zones::ZoneType;

    fn interior_id_for(world_x: i64, world_z: i64, seed: u64) -> InteriorId {
        hash_coords(world_x, world_z, seed, domain::INTERIOR)
    }

    /// A residential home-shaped context used across placeholder/generation tests.
    fn home_ctx(id: InteriorId, seed: u64) -> InteriorContext {
        InteriorContext::new(
            id,
            ZoneType::Residential,
            [0.0; 5],
            8.0,
            4.0,
            64,
            8,
            8,
            1,
            seed,
        )
    }

    /// A tall downtown skyscraper-shaped context (many floors).
    fn tower_ctx(id: InteriorId, seed: u64) -> InteriorContext {
        InteriorContext::new(
            id,
            ZoneType::Downtown,
            [0.0; 5],
            120.0,
            4.0,
            64,
            12,
            12,
            2,
            seed,
        )
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
        let ctx = home_ctx(42, 445566);
        let a = PlaceholderInteriorState::generate(42, &ctx);
        let b = PlaceholderInteriorState::generate(42, &ctx);
        assert_eq!(a, b);
        assert_eq!(a.id, 42);
        assert_eq!(a.seed, 445566);
        // Non-null placeholder: footprint echoed into width/height, floors set.
        assert_eq!(a.width, 8);
        assert_eq!(a.height, 8);
        assert_eq!(a.floors, 2); // 8u / 4u per floor, ceil = 2
    }

    #[test]
    fn placeholder_uses_context_footprint_over_config_range() {
        // A context with a set footprint must win over the config's 6..14 range.
        let ctx = home_ctx(5, 99);
        let s = PlaceholderInteriorState::generate(5, &ctx);
        assert_eq!(s.width, 8);
        assert_eq!(s.height, 8);
        assert_eq!(s.palette_id, ctx.palette_id);
    }

    #[test]
    fn different_contexts_produce_different_placeholders() {
        let a = PlaceholderInteriorState::generate(123, &home_ctx(123, 1));
        let b = PlaceholderInteriorState::generate(123, &home_ctx(123, 2));
        assert_ne!(a, b, "different seeds should differ");
        // Also via free function.
        let c: PlaceholderInteriorState = generate_interior(999, &home_ctx(999, 10));
        let d: PlaceholderInteriorState = generate_interior(999, &home_ctx(999, 11));
        assert_ne!(c, d);
    }

    #[test]
    fn baseline_layout_is_deterministic_and_walled() {
        let ctx = tower_ctx(7, 42);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let a = generate_layout(7, &ctx, &bp);
        let b = generate_layout(7, &ctx, &bp);
        assert_eq!(a, b);
        // A 120u / 4u tower yields 30 floors, each walled on its outer ring.
        assert_eq!(a.floors.len(), 30);
        for floor in &a.floors {
            // Corners are exterior walls.
            assert_eq!(floor.tile(0, 0), Tile::Wall);
            assert_eq!(floor.tile(floor.width - 1, floor.depth - 1), Tile::Wall);
            // Interior must contain a core (circulation) for a tall building.
            assert!(
                floor.tiles.contains(&Tile::Core),
                "tower floor missing circulation core"
            );
        }
        assert_eq!(a.id, 7);
        assert_eq!(a.seed, 42);
    }

    #[test]
    fn residential_has_fewer_floors_than_downtown() {
        let home = generate_layout(
            1,
            &home_ctx(1, 5),
            &blueprint_defaults(ZoneType::Residential),
        );
        let tower = generate_layout(2, &tower_ctx(2, 5), &blueprint_defaults(ZoneType::Downtown));
        assert_eq!(home.floors.len(), 2);
        assert_eq!(tower.floors.len(), 30);
        assert!(home.floors.len() < tower.floors.len());
    }

    #[test]
    fn interior_cache_basic() {
        let mut cache = InteriorCache::<PlaceholderInteriorState>::new(2);
        let s1 = PlaceholderInteriorState::generate(1, &home_ctx(1, 10));
        let s2 = PlaceholderInteriorState::generate(2, &home_ctx(2, 10));
        let s3 = PlaceholderInteriorState::generate(3, &home_ctx(3, 10));
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
        cache.insert(42, PlaceholderInteriorState::generate(42, &home_ctx(42, 1)));
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
    }
}
