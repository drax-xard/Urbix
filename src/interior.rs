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
//! doors) is driven by the per-zone [`crate::layout::Blueprint`] tables, and the
//! whole floor is posed from the context (zone, floors, footprint, entrance
//! side) deterministically, so a skyscraper, a home, and a warehouse read
//! differently.

use std::collections::HashMap;

use crate::config::WorldConfig;
use crate::data::InteriorId;
use crate::hash::{domain, hash_coords, hash_unit};
use crate::layout::{
    Blueprint, BlueprintRoom, DoorSide, Floor, InteriorContext, InteriorLayout, Tile,
};

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
/// use urbix::layout::{DoorSide, InteriorContext, blueprint_defaults};
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
///     door_side: DoorSide::West,
///     seed: 445566,
/// };
/// let state = generate_interior::<PlaceholderInteriorState>(42, &ctx);
/// assert_eq!(state.id, 42);
/// ```
#[must_use]
pub fn generate_interior<S: InteriorState>(id: InteriorId, ctx: &InteriorContext) -> S {
    S::generate(id, ctx)
}

/// Generate a deterministic, walled [`InteriorLayout`] from an exterior context
/// and a zone blueprint (Milestone 9).
///
/// Turns a lot's context (zone, floors, footprint, entrance side) plus its
/// blueprint into one [`Floor`] grid per storey. Each floor is carved as:
///
/// 1. A sealed ring of exterior `Wall` tiles (nothing leaks out of the
///    footprint).
/// 2. A hashed vertical-circulation `Core` square (stairs/elevator), sized by
///    `blueprint.core_size`.
/// 3. A street-facing entrance: a `Door` on the lot edge recorded in
///    `ctx.door_side`.
/// 4. Rooms rolled from `blueprint.room_slice()` (weighted by each template's
///    `weight`, sized within its `min`/`max` bounds) and placed greedily
///    against the free area, each with a one-tile margin so it opens onto a
///    corridor channel. If the rolled template still doesn't fit (cramped
///    lots), placement retries the blueprint's smallest template.
/// 5. Every leftover free cell filled with `Corridor`, and a single `Door`
///    punched on each room's boundary where it meets circulation.
///
/// Everything is a pure function of `hash(id, floor, seed, domain)`, so the
/// same lot always yields the same interiors and distinct storeys/seeds differ.
#[must_use]
pub fn generate_layout(
    id: InteriorId,
    ctx: &InteriorContext,
    blueprint: &Blueprint,
) -> InteriorLayout {
    let seed = ctx.seed;
    let (x_id, y_id) = split_id(id);

    // Number of floors defaults to the context; a degenerate footprint still
    // yields a usable single (sealed) floor so the result is never empty.
    let floor_count = ctx.floor_count.max(1);
    let floors = (0..floor_count)
        .map(|f| generate_floor(x_id, y_id, seed, f, ctx, blueprint))
        .collect::<Vec<_>>();

    InteriorLayout {
        id,
        seed,
        context: *ctx,
        floors,
    }
}

/// Generate the `floor`-th storey of an interior.
///
/// See [`generate_layout`] for the full carving pipeline. `x_id`/`y_id` are the
/// interior id's coordinate halves and `seed` the world seed; every draw mixes
/// in the floor number so adjacent storeys vary independently.
#[must_use]
fn generate_floor(
    x_id: i64,
    y_id: i64,
    seed: u64,
    floor: u8,
    ctx: &InteriorContext,
    blueprint: &Blueprint,
) -> Floor {
    let mut g = Floor::empty(ctx.footprint_w, ctx.footprint_d);
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);

    // Degenerate footprint (no interior): seal it solid so an unwalled void is
    // never exposed.
    if g.width < 3 || g.depth < 3 {
        g.tiles.fill(Tile::Wall);
        return g;
    }

    paint_wall_ring(&mut g);

    // Vertical circulation core: a filled square whose placement is hashed per
    // floor, clamped inside the wall ring and one tile off every wall. The core
    // is also capped so it never swallows a small lot's interior: at most about
    // half the inner area, leaving room for at least one room and circulation.
    let core = blueprint
        .core_size
        .max(1)
        .min(((gw.saturating_sub(2) as u8) / 2).max(1));
    let inner_w = (gw.saturating_sub(usize::from(core) + 1)).max(1);
    let inner_d = (gd.saturating_sub(usize::from(core) + 1)).max(1);
    let core_base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_FLOOR);
    let cx = 1u8 + (core_base % inner_w as u64) as u8;
    let cz = 1u8 + ((core_base >> 16) % inner_d as u64) as u8;
    paint_core(&mut g, cx, cz, core);

    // Street-facing entrance on the lot edge recorded in the context. The cell
    // just inside the door is reserved as corridor (rooms may hug it but never
    // claim it), so the street access always connects to the interior.
    let entrance_base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_DOOR);
    punch_entrance_door(&mut g, ctx.door_side, entrance_base);
    reserve_entrance(&mut g, ctx.door_side, entrance_base);

    // Weighted room placement against the free area.
    let rooms = blueprint.room_slice();
    if !rooms.is_empty() {
        let room_base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_ROOM);

        // Candidate anchors: every free interior cell, visited in a
        // deterministic pseudo-random order (Fisher–Yates on the hash stream).
        let mut anchors: Vec<(u8, u8)> = Vec::new();
        for z in 1..gd - 1 {
            for x in 1..gw - 1 {
                if g.tiles[z * gw + x] == Tile::Void {
                    anchors.push((x as u8, z as u8));
                }
            }
        }
        for i in (1..anchors.len()).rev() {
            let j = pick(room_base, i, i + 1);
            anchors.swap(i, j);
        }

        let mut placed: Vec<(u8, u8, u8, u8)> = Vec::new();
        for (k, (ax, az)) in anchors.into_iter().enumerate() {
            // Bound the work on very large footprints; 128 fills any sane home.
            if k >= 128 {
                break;
            }
            let idx = usize::from(az) * gw + usize::from(ax);
            if g.tiles[idx] != Tile::Void {
                continue;
            }
            let roll = unit_draw(room_base, k);
            let mut room = roll_room(rooms, roll);
            let mut rect = try_place_room(&mut g, ax, az, room, room_base, k);
            // Fallback: on a cramped lot the rolled template is often too large
            // for the remaining interior. Retry once with the smallest template
            // so small lots always get rooms instead of corridor-only shells.
            if rect.is_none() {
                let smallest = rooms
                    .iter()
                    .min_by_key(|r| u16::from(r.min_w) * u16::from(r.min_d));
                if let Some(s) = smallest {
                    if !std::ptr::eq(room, s) {
                        room = s;
                        rect = try_place_room(&mut g, ax, az, room, room_base, k);
                    }
                }
            }
            if let Some(rect) = rect {
                paint_room(&mut g, rect.0, rect.1, rect.2, rect.3, room.kind);
                placed.push(rect);
            }
        }

        // A door from each room onto circulation (k = 1 draws independently of
        // the entrance pick at k = 0 on the same door stream).
        let door_base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_DOOR);
        for rect in placed {
            room_door(&mut g, rect, door_base);
        }
    }

    // Corridor fill: every leftover free cell becomes circulation, so the
    // one-tile margin around each room (and around the core) is a navigable
    // channel connecting everything, including the street-facing entrance.
    for z in 1..gd - 1 {
        for x in 1..gw - 1 {
            let idx = z * gw + x;
            if g.tiles[idx] == Tile::Void {
                g.tiles[idx] = Tile::Corridor;
            }
        }
    }

    g
}

/// Paint the sealed outer wall ring of a floor grid.
fn paint_wall_ring(g: &mut Floor) {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let last_w = gw - 1;
    let last_d = gd - 1;
    for x in 0..=last_w {
        g.tiles[x] = Tile::Wall;
        g.tiles[last_d * gw + x] = Tile::Wall;
    }
    for z in 0..=last_d {
        g.tiles[z * gw] = Tile::Wall;
        g.tiles[z * gw + last_w] = Tile::Wall;
    }
}

/// Place the street-facing entrance: a single `Door` on the outer wall ring on
/// `side`, at a hashed offset along that edge. The interior beside the door is
/// guaranteed to become `Corridor` (rooms keep a margin off the wall), so the
/// entrance always connects to circulation.
fn punch_entrance_door(g: &mut Floor, side: DoorSide, base: u64) {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let idx = match side {
        DoorSide::West => {
            let z = 1 + pick(base, 0, gd - 2);
            z * gw
        }
        DoorSide::East => {
            let z = 1 + pick(base, 0, gd - 2);
            z * gw + gw - 1
        }
        DoorSide::North => 1 + pick(base, 0, gw - 2),
        DoorSide::South => {
            let x = 1 + pick(base, 0, gw - 2);
            (gd - 1) * gw + x
        }
    };
    g.tiles[idx] = Tile::Door;
    g.kinds[idx] = 0;
}

/// Reserve the interior cell directly behind the entrance `Door` as `Corridor`,
/// using the same hashed offset as [`punch_entrance_door`] so both agree on
/// which cell is the doorway.
fn reserve_entrance(g: &mut Floor, side: DoorSide, base: u64) {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let idx = match side {
        DoorSide::West => {
            let z = 1 + pick(base, 0, gd - 2);
            z * gw + 1
        }
        DoorSide::East => {
            let z = 1 + pick(base, 0, gd - 2);
            z * gw + (gw - 2)
        }
        DoorSide::North => {
            let x = 1 + pick(base, 0, gw - 2);
            gw + x
        }
        DoorSide::South => {
            let x = 1 + pick(base, 0, gw - 2);
            (gd - 2) * gw + x
        }
    };
    g.tiles[idx] = Tile::Corridor;
    g.kinds[idx] = 0;
}

/// Whether a `w×d` room with top-left corner `(x0, z0)` fits: the rectangle
/// itself must be free `Void`, and every cell in a one-tile margin around it
/// must not be an already-placed room. The margin is what becomes the corridor
/// channel keeping each room reachable. Exterior `Wall` (rooms may hug the
/// block perimeter), the circulation `Core` (a room can open straight onto
/// it), and an entrance `Door` are all valid margin neighbours.
fn room_fits(g: &Floor, x0: i64, z0: i64, w: u8, d: u8) -> bool {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let (w, d) = (i64::from(w), i64::from(d));
    for z in z0..z0 + d {
        for x in x0..x0 + w {
            if x < 0 || z < 0 || x >= gw as i64 || z >= gd as i64 {
                return false;
            }
            if g.tiles[z as usize * gw + x as usize] != Tile::Void {
                return false;
            }
        }
    }
    for z in z0 - 1..=z0 + d {
        for x in x0 - 1..=x0 + w {
            if x < 0 || z < 0 || x >= gw as i64 || z >= gd as i64 {
                continue;
            }
            if g.tiles[z as usize * gw + x as usize] == Tile::Room {
                return false;
            }
        }
    }
    true
}

/// Try to place `room` with top-left at `(ax, az)`, rolling a size within the
/// template's bounds and falling back to the closest fit. On success paints the
/// room and returns its rect `(x0, z0, w, d)`.
fn try_place_room(
    g: &mut Floor,
    ax: u8,
    az: u8,
    room: &BlueprintRoom,
    base: u64,
    k: usize,
) -> Option<(u8, u8, u8, u8)> {
    if room.max_w == 0 || room.max_d == 0 {
        return None;
    }
    let span_w = u32::from(room.max_w - room.min_w) + 1;
    let span_d = u32::from(room.max_d - room.min_d) + 1;
    let tw = room.min_w + (unit_draw(base, k * 2) * span_w as f32) as u8;
    let td = room.min_d + (unit_draw(base, k * 2 + 1) * span_d as f32) as u8;

    // Try every size in the template bounds, closest to the roll first.
    let mut combos: Vec<(u8, u8)> = (room.min_w..=room.max_w)
        .flat_map(|w| (room.min_d..=room.max_d).map(move |d| (w, d)))
        .collect();
    combos.sort_by_key(|&(w, d)| {
        (
            i64::from(w).abs_diff(i64::from(tw)) + i64::from(d).abs_diff(i64::from(td)),
            w,
            d,
        )
    });
    for (w, d) in combos {
        if room_fits(g, i64::from(ax), i64::from(az), w, d) {
            return Some((ax, az, w, d));
        }
    }
    None
}

/// Stamp a `w×d` rect of tiles as `Room` carrying `kind`.
fn paint_room(g: &mut Floor, x0: u8, z0: u8, w: u8, d: u8, kind: u8) {
    let gw = usize::from(g.width);
    for z in usize::from(z0)..usize::from(z0) + usize::from(d) {
        for x in usize::from(x0)..usize::from(x0) + usize::from(w) {
            let i = z * gw + x;
            g.tiles[i] = Tile::Room;
            g.kinds[i] = kind;
        }
    }
}

/// Punch a single `Door` on a room's boundary: walk the room's perimeter in a
/// deterministic rotation of the hash stream and turn the first margin cell the
/// room faces into a `Door` (the margin is pure circulation by construction).
/// The door lives in the corridor channel, so the room itself stays intact, the
/// opening always leads to circulation, and nested rooms of any size remain
/// fully walled.
fn room_door(g: &mut Floor, rect: (u8, u8, u8, u8), base: u64) {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let (x0, z0, w, d) = (
        usize::from(rect.0),
        usize::from(rect.1),
        usize::from(rect.2),
        usize::from(rect.3),
    );
    let mut perimeter: Vec<usize> = Vec::with_capacity(2 * (w + d));
    for x in x0..x0 + w {
        perimeter.push(z0 * gw + x);
        perimeter.push((z0 + d - 1) * gw + x);
    }
    for z in z0 + 1..z0 + d - 1 {
        perimeter.push(z * gw + x0);
        perimeter.push(z * gw + x0 + w - 1);
    }
    if perimeter.is_empty() {
        return;
    }
    let rot = pick(base, 1, perimeter.len());
    perimeter.rotate_left(rot);
    for idx in perimeter {
        let (x, z) = (idx % gw, idx / gw);
        let (x, z) = (x as i64, z as i64);
        for (nx, nz) in [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)] {
            if nx < 0 || nz < 0 || nx >= gw as i64 || nz >= gd as i64 {
                continue;
            }
            let ni = nz as usize * gw + nx as usize;
            match g.tiles[ni] {
                // The margin cell touching the room becomes the doorway; it may
                // already be a door punched for the street access.
                Tile::Corridor | Tile::Core | Tile::Door => {
                    g.tiles[ni] = Tile::Door;
                    g.kinds[ni] = 0;
                    return;
                }
                _ => {}
            }
        }
    }
}

/// Hash for one storey: folds `floor` into the coordinate stream so each level
/// of a building draws independently of the others.
fn floor_hash(x: i64, y: i64, seed: u64, floor: u8, domain: u8) -> u64 {
    hash_coords(
        x ^ i64::from(floor).wrapping_mul(0x9e37_79b9_7f4a_7c15u64 as i64),
        y,
        seed,
        domain,
    )
}

/// A deterministic unit draw in `[0, 1)` at index `k` of a `base` hash stream.
fn unit_draw(base: u64, k: usize) -> f32 {
    hash_unit(base as i64, k as i64, 0, 0)
}

/// A deterministic index into `[0, n)` using a `base` hash stream and index `k`.
fn pick(base: u64, k: usize, n: usize) -> usize {
    if n == 0 {
        0
    } else {
        (hash_coords(base as i64, k as i64, 0, 0) % n as u64) as usize
    }
}

/// Weighted room roll over the blueprint's live room templates.
///
/// The roll is a unit draw; the template whose cumulative weight first reaches
/// it is chosen, so higher-`weight` templates are picked proportionally more
/// often. Non-positive weights never win; an all-non-positive table falls back
/// to the first template.
fn roll_room(rooms: &[BlueprintRoom], roll: f32) -> &BlueprintRoom {
    debug_assert!(!rooms.is_empty());
    let total: f32 = rooms.iter().map(|r| r.weight.max(0.0)).sum();
    if total <= 0.0 {
        return &rooms[0];
    }
    let mut t = roll.clamp(0.0, 0.999_999_9) * total;
    for r in rooms {
        t -= r.weight.max(0.0);
        if t <= 0.0 {
            return r;
        }
    }
    &rooms[rooms.len() - 1]
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
    use crate::layout::{blueprint_defaults, DoorSide, InteriorContext};
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
            DoorSide::West,
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
            DoorSide::East,
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

    /// An 8×8 residential two-storey lot, built on an arbitrary entrance side.
    fn lot_ctx(id: InteriorId, side: DoorSide, seed: u64) -> InteriorContext {
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
            side,
            seed,
        )
    }

    #[test]
    fn layout_is_deterministic_and_walled() {
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
            assert!(
                floor.tiles.contains(&Tile::Door),
                "tower floor missing an entrance/room door"
            );
        }
        assert_eq!(a.id, 7);
        assert_eq!(a.seed, 42);
    }

    #[test]
    fn rooms_are_placed_walled_sealed_and_reachable() {
        let ctx = lot_ctx(42, DoorSide::West, 445566);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let layout = generate_layout(42, &ctx, &bp);
        assert_eq!(layout.floors.len(), 2);

        for floor in &layout.floors {
            let gw = usize::from(floor.width);
            let gd = usize::from(floor.depth);
            // Sealed: the outer ring is exterior wall, except for the
            // street-facing entrance (a deliberate Door opening).
            for x in 0..gw {
                assert!(
                    matches!(floor.tiles[x], Tile::Wall | Tile::Door),
                    "top edge not sealed"
                );
                assert!(
                    matches!(floor.tiles[(gd - 1) * gw + x], Tile::Wall | Tile::Door),
                    "bottom edge not sealed"
                );
            }
            for z in 0..gd {
                assert!(matches!(floor.tiles[z * gw], Tile::Wall | Tile::Door));
                assert!(matches!(
                    floor.tiles[z * gw + gw - 1],
                    Tile::Wall | Tile::Door
                ));
            }
            // Rooms exist, every room as a whole is reachable from circulation
            // (a room may be larger than a corridor can border on all sides
            // internally, so reachability is a flood fill through the room's
            // own tiles), and kind tags are only set on room tiles (and match
            // the residential blueprint).
            let total = gw * gd;
            let mut visited = vec![false; total];
            let mut rooms = 0usize;
            for start in 0..total {
                if visited[start] || floor.tiles[start] != Tile::Room {
                    continue;
                }
                rooms += 1;
                let mut stack = vec![start];
                visited[start] = true;
                let mut touches_circulation = false;
                while let Some(i) = stack.pop() {
                    let x = i % gw;
                    let z = i / gw;
                    for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                        let nx = x as i64 + dx;
                        let nz = z as i64 + dz;
                        if nx < 0 || nz < 0 || nx >= gw as i64 || nz >= gd as i64 {
                            continue;
                        }
                        let ni = nz as usize * gw + nx as usize;
                        match floor.tiles[ni] {
                            Tile::Room if !visited[ni] => {
                                visited[ni] = true;
                                stack.push(ni);
                            }
                            Tile::Corridor | Tile::Door | Tile::Core => {
                                touches_circulation = true;
                            }
                            _ => {}
                        }
                    }
                }
                assert!(
                    touches_circulation,
                    "room starting at ({},{}) is isolated",
                    start % gw,
                    start / gw
                );
            }
            assert!(rooms > 0, "no rooms placed on this floor");
            for i in 0..total {
                if floor.tiles[i] == Tile::Room {
                    assert!(
                        (20..=23).contains(&floor.kinds[i]),
                        "room kind {} not from the residential blueprint",
                        floor.kinds[i]
                    );
                } else {
                    assert_eq!(
                        floor.kinds[i], 0,
                        "kind set on non-room tile {:?}",
                        floor.tiles[i]
                    );
                }
            }
        }
    }

    #[test]
    fn interiors_vary_across_floors_and_seeds() {
        let bp = crate::layout::blueprint_defaults(ZoneType::Downtown);
        let tower = generate_layout(9, &tower_ctx(9, 42), &bp);
        assert!(tower.floors.len() >= 2);
        let all_same = tower
            .floors
            .windows(2)
            .all(|pair| pair[0].tiles == pair[1].tiles);
        assert!(!all_same, "every storey of the tower is identical");

        let a = generate_layout(
            1,
            &home_ctx(1, 1),
            &crate::layout::blueprint_defaults(ZoneType::Residential),
        );
        let b = generate_layout(
            1,
            &home_ctx(1, 2),
            &crate::layout::blueprint_defaults(ZoneType::Residential),
        );
        assert_ne!(
            a, b,
            "different world seeds must generate different interiors"
        );
    }

    #[test]
    fn entrance_door_faces_the_context_side() {
        let bp = crate::layout::blueprint_defaults(ZoneType::Residential);
        for side in [
            DoorSide::West,
            DoorSide::East,
            DoorSide::North,
            DoorSide::South,
        ] {
            let floor = &generate_layout(1, &lot_ctx(7, side, 99), &bp).floors[0];
            let gw = usize::from(floor.width);
            let gd = usize::from(floor.depth);
            let hits = |pred: &dyn Fn(usize) -> bool| {
                floor
                    .tiles
                    .iter()
                    .enumerate()
                    .any(|(i, t)| *t == Tile::Door && pred(i))
            };
            match side {
                DoorSide::West => assert!(hits(&|i| i % gw == 0)),
                DoorSide::East => assert!(hits(&|i| i % gw == gw - 1)),
                DoorSide::North => assert!(hits(&|i| i / gw == 0)),
                DoorSide::South => assert!(hits(&|i| i / gw == gd - 1)),
            }
        }
    }

    #[test]
    fn small_lot_still_gets_rooms() {
        // Downtown's raw street block is 4 cells; the context bridge floors the
        // footprint at 7x7 and the core is capped to half the interior, so even
        // the smallest practical lot keeps a room window open on every storey.
        let ctx = InteriorContext::new(
            7,
            ZoneType::Downtown,
            [0.0; 5],
            16.0,
            4.0,
            64,
            7,
            7,
            2,
            DoorSide::West,
            42,
        );
        let layout = generate_layout(7, &ctx, &blueprint_defaults(ZoneType::Downtown));
        assert!(!layout.floors.is_empty());
        for floor in &layout.floors {
            assert!(
                floor.tiles.contains(&Tile::Room),
                "7x7 lot floor has no rooms"
            );
        }
    }

    #[test]
    fn degenerate_footprint_is_sealed() {
        let ctx = InteriorContext::new(
            1,
            ZoneType::Park,
            [0.0; 5],
            8.0,
            4.0,
            64,
            2,
            2,
            0,
            DoorSide::North,
            7,
        );
        let layout = generate_layout(1, &ctx, &crate::layout::blueprint_defaults(ZoneType::Park));
        for floor in &layout.floors {
            assert!(floor.tiles.iter().all(|t| *t == Tile::Wall));
        }
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
