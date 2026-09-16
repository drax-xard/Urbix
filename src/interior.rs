//! # interior.rs
//!
//! Interior generation surface for the Urbix engine.
//!
//! Every built lot is assigned a stable [`crate::data::InteriorId`] during
//! chunk generation (`chunk.rs:interior_id_for_lot`, one key per lot). This module defines the
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
    Blueprint, BlueprintRoom, DoorSide, Floor, InteriorContext, InteriorLayout, Tile, TAG_WET,
};
use crate::zones::{ZoneType, ZONE_COUNT};

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
///     corner: false,
///     frontage_depth: 8,
///     building_role: urbix::layout::BuildingRole::Ordinary,
///     secondary_zone: ZoneType::Commercial,
/// };
/// let state = generate_interior::<PlaceholderInteriorState>(42, &ctx);
/// assert_eq!(state.id, 42);
/// ```
#[must_use]
pub fn generate_interior<S: InteriorState>(id: InteriorId, ctx: &InteriorContext) -> S {
    S::generate(id, ctx)
}

/// Generate a deterministic, walled [`InteriorLayout`] from an exterior context
/// and a zone blueprint (Milestone 9; vertically structured in Milestone 13).
///
/// Turns a lot's context (zone, floors, lot-rect footprint, entrance side,
/// building role) plus its blueprint into one [`Floor`] grid per storey.
/// Storeys carry roles ([`FloorRole`]): floor 0 is Ground (street entrance,
/// lobby halo), the last is Top (expanded mechanical core), and Typical
/// floors between repeat one layout — generated once and cloned unless the
/// blueprint sets `vary_typical`. The circulation core stacks vertically at
/// one hashed position (building-level `domain::LAYOUT_CORE` draw) unless
/// the blueprint opts into `wandering_core`. Each floor is otherwise carved
/// as in Milestone 9:
///
/// 1. A sealed ring of exterior `Wall` tiles (nothing leaks out of the
///    footprint).
/// 2. The stacked (or wandering) `Core` square, sized by
///    `blueprint.core_size` (+1 on Top).
/// 3. Ground: street entrance `Door` on `ctx.door_side` plus a lobby halo of
///    `Corridor` around the core. Above ground: a lobby `Door` on the core
///    edge instead of a street door.
/// 4. Rooms rolled from `blueprint.room_slice()` (weighted by each template's
///    `weight`, sized within its `min`/`max` bounds) and placed greedily
///    against the free area, each with a one-tile margin so it opens onto a
///    corridor channel. If the rolled template still doesn't fit (cramped
///    lots), placement retries the blueprint's smallest template.
/// 5. Every leftover free cell filled with `Corridor`, and a single `Door`
///    punched on each room's boundary where it meets circulation.
///
/// Everything is a pure function of `hash(id, floor, seed, domain)`, so the
/// same lot always yields the same interiors and distinct buildings differ.
///
/// Mixed-use buildings resolve their ground floor through
/// [`generate_layout_with_ground`]; this entry point uses `blueprint` for
/// every storey.
#[must_use]
pub fn generate_layout(
    id: InteriorId,
    ctx: &InteriorContext,
    blueprint: &Blueprint,
) -> InteriorLayout {
    generate_layout_with_ground(id, ctx, blueprint, blueprint)
}

/// Resolve the ground-floor blueprint for mixed-use buildings.
///
/// When the building has more than two storeys and its blueprint names a
/// `ground_zone` override (e.g. homes default to a Commercial retail base),
/// floor 0 uses that zone's table; otherwise the main blueprint serves every
/// floor. Pure data lookup — deterministic by construction.
#[must_use]
pub fn resolve_ground_override<'a>(
    ctx: &InteriorContext,
    blueprint: &'a Blueprint,
    table: &'a [Blueprint; ZONE_COUNT],
) -> &'a Blueprint {
    if ctx.floor_count > 2 && blueprint.ground_zone != 255 {
        if let Some(z) = ZoneType::all()
            .iter()
            .find(|z| **z as u8 == blueprint.ground_zone)
        {
            return &table[*z as usize];
        }
    }
    blueprint
}

/// Generate a layout with an explicit ground-floor blueprint.
///
/// Identical to [`generate_layout`] except floor 0 (when one exists) draws
/// its rooms, units, and shafts from `ground` instead of `blueprint`. Core
/// position, entrance side, and floor roles still come from `ctx`; the shaft
/// stacks through the retail base like a real mixed-use tower.
#[must_use]
pub fn generate_layout_with_ground(
    id: InteriorId,
    ctx: &InteriorContext,
    blueprint: &Blueprint,
    ground: &Blueprint,
) -> InteriorLayout {
    use crate::layout::{floor_role, FloorRole};
    let seed = ctx.seed;
    let (x_id, y_id) = split_id(id);

    // Number of floors defaults to the context; a degenerate footprint still
    // yields a usable single (sealed) floor so the result is never empty.
    let floor_count = ctx.floor_count.max(1);
    let gw = usize::from(ctx.footprint_w.max(1));
    let gd = usize::from(ctx.footprint_d.max(1));

    // Stacked shaft: one core position per building (no floor fold), shared
    // by every storey so stairs/elevators run vertically. Wander opt-in
    // (`wandering_core`) falls back to per-floor draws in the loop below.
    let stacked = if blueprint.wandering_core == 0 {
        Some(stacked_core_rect(
            x_id,
            y_id,
            seed,
            gw,
            gd,
            blueprint.core_size,
        ))
    } else {
        None
    };

    // Typical program generates once and clones: a tower's middle floors are
    // one repeated layout, like a real typical plan (and ~N× cheaper).
    // Plumbing shafts are building-level (main blueprint), constant on every
    // floor including a mixed-use ground, so wet stacks never jog.
    let shafts = wet_shaft_columns(x_id, y_id, seed, gw, blueprint.wet_shafts);
    let has_typical = floor_count > 2;
    let typical = if has_typical && blueprint.vary_typical == 0 {
        let (csize, cx, cz) = stacked.unwrap_or_else(|| {
            core_rect_for_floor(x_id, y_id, seed, 1, gw, gd, blueprint.core_size)
        });
        Some(generate_floor(
            x_id,
            y_id,
            seed,
            1,
            FloorRole::Typical,
            (csize, cx, cz),
            ctx,
            blueprint,
            &shafts,
        ))
    } else {
        None
    };

    let mut floors = Vec::with_capacity(usize::from(floor_count));
    for f in 0..floor_count {
        let role = floor_role(f, floor_count);
        if role == FloorRole::Typical {
            if let Some(t) = typical.clone() {
                floors.push(t);
                continue;
            }
        }
        // Mixed-use ground: floor 0 draws rooms/units from the override
        // table; the stacked shaft still comes from the main blueprint.
        let bp = if f == 0 { ground } else { blueprint };
        let (csize, cx, cz) = stacked.unwrap_or_else(|| {
            core_rect_for_floor(x_id, y_id, seed, f, gw, gd, blueprint.core_size)
        });
        // Top floors expand the mechanical core by one tile.
        let csize = if role == FloorRole::Top {
            csize
                .saturating_add(1)
                .min(ctx.footprint_w.max(1))
                .min(ctx.footprint_d.max(1))
        } else {
            csize
        };
        floors.push(generate_floor(
            x_id,
            y_id,
            seed,
            f,
            role,
            (csize, cx, cz),
            ctx,
            bp,
            &shafts,
        ));
    }

    InteriorLayout {
        id,
        seed,
        context: *ctx,
        floors,
    }
}

/// Cap a core size to the footprint so the shaft never swallows a small
/// lot's interior: at most about half the inner area, leaving room for at
/// least one room and circulation.
fn cap_core_size(requested: u8, gw: usize, gd: usize) -> u8 {
    requested
        .max(1)
        .min(((gw.saturating_sub(2) as u8) / 2).max(1))
        .min(((gd.saturating_sub(2) as u8) / 2).max(1))
}

/// Place a `size` shaft on a `gw×gd` grid from a hash `base`: returns the
/// top-left `(x, z)`, clamped inside the wall ring. Shared by the stacked
/// and wandering paths so both obey the same geometry.
fn place_core(gw: usize, gd: usize, size: u8, base: u64) -> (u8, u8) {
    let inner_w = (gw.saturating_sub(usize::from(size) + 1)).max(1);
    let inner_d = (gd.saturating_sub(usize::from(size) + 1)).max(1);
    let cx = 1u8 + (base % inner_w as u64) as u8;
    let cz = 1u8 + ((base >> 16) % inner_d as u64) as u8;
    (cx, cz)
}

/// One shaft rect `(size, x, z)` per building: no floor fold, so every
/// storey shares the same vertical circulation.
fn stacked_core_rect(
    x_id: i64,
    y_id: i64,
    seed: u64,
    gw: usize,
    gd: usize,
    requested: u8,
) -> (u8, u8, u8) {
    let size = cap_core_size(requested, gw, gd);
    let base = hash_coords(x_id, y_id, seed, domain::LAYOUT_CORE);
    let (cx, cz) = place_core(gw, gd, size, base);
    (size, cx, cz)
}

/// One shaft rect `(size, x, z)` for a single wandering floor: the legacy
/// per-floor draw, kept for blueprints that opt into `wandering_core`.
fn core_rect_for_floor(
    x_id: i64,
    y_id: i64,
    seed: u64,
    floor: u8,
    gw: usize,
    gd: usize,
    requested: u8,
) -> (u8, u8, u8) {
    let size = cap_core_size(requested, gw, gd);
    let base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_FLOOR);
    let (cx, cz) = place_core(gw, gd, size, base);
    (size, cx, cz)
}

/// Paint a one-tile lobby halo of `Corridor` around the shaft rect (ground
/// floor only, after rooms). Only `Void` cells convert, so walls, doors,
/// rooms, and the shaft itself are untouched — rooms placed earlier keep
/// their cells, and the halo just opens whatever space is left.
fn paint_lobby_halo(g: &mut Floor, cx: u8, cz: u8, size: u8) {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let (cx, cz, size) = (i64::from(cx), i64::from(cz), i64::from(size));
    for z in cz - 1..=cz + size {
        for x in cx - 1..=cx + size {
            if x < 1 || z < 1 || x + 1 >= gw as i64 || z + 1 >= gd as i64 {
                continue;
            }
            let idx = z as usize * gw + x as usize;
            if g.tiles[idx] == Tile::Void {
                g.tiles[idx] = Tile::Corridor;
            }
        }
    }
}

/// Punch a lobby `Door` on the shaft edge: the first `Void` cell facing the
/// shaft (row-major scan) becomes the elevator/stair door, so upper floors
/// open onto circulation without a street door. If the shaft is fully
/// enclosed (cramped lots), nothing happens — corridor fill still reaches it.
fn punch_core_door(g: &mut Floor, cx: u8, cz: u8, size: u8) {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let (cx, cz, size) = (usize::from(cx), usize::from(cz), usize::from(size));
    for z in cz.saturating_sub(1)..=(cz + size).min(gd.saturating_sub(1)) {
        for x in cx.saturating_sub(1)..=(cx + size).min(gw.saturating_sub(1)) {
            // Only cells orthogonally facing a shaft tile qualify (no corner
            // doors to nowhere).
            let above_below = (z + 1 == cz || z == cz + size) && x >= cx && x < cx + size;
            let left_right = (x + 1 == cx || x == cx + size) && z >= cz && z < cz + size;
            if !(above_below || left_right) {
                continue;
            }
            if x == 0 || z == 0 || x + 1 >= gw || z + 1 >= gd {
                continue;
            }
            let idx = z * gw + x;
            if g.tiles[idx] == Tile::Void {
                g.tiles[idx] = Tile::Door;
                g.kinds[idx] = 0;
                return;
            }
        }
    }
}

/// A rectangular placement region inside a floor grid (grid coordinates).
///
/// Rooms never cross region bounds: open-plan floors use the whole interior,
/// single-loaded floors exclude the corridor band, and apartment units each
/// get their own region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlaceRegion {
    /// Left edge in grid cells.
    x0: u8,
    /// Top edge in grid cells.
    z0: u8,
    /// Width in cells.
    w: u8,
    /// Depth in cells.
    d: u8,
}

impl PlaceRegion {
    /// The whole interior inside the wall ring of a `gw×gd` grid.
    fn interior(gw: usize, gd: usize) -> Self {
        Self {
            x0: 1,
            z0: 1,
            w: gw.saturating_sub(2) as u8,
            d: gd.saturating_sub(2) as u8,
        }
    }

    /// Whether a `w×d` rect at `(x, z)` lies fully inside the region.
    fn contains(&self, x: i64, z: i64, w: u8, d: u8) -> bool {
        let (rx0, rz0) = (i64::from(self.x0), i64::from(self.z0));
        x >= rx0
            && z >= rz0
            && x + i64::from(w) <= rx0 + i64::from(self.w)
            && z + i64::from(d) <= rz0 + i64::from(self.d)
    }
}

/// A placed room: rect plus the door count its template requires.
struct PlacedRoom {
    /// Top-left x in grid cells.
    x0: u8,
    /// Top-left z in grid cells.
    z0: u8,
    /// Width in cells.
    w: u8,
    /// Depth in cells.
    d: u8,
    /// Doors to punch for this room.
    doors: u8,
}

/// Plumbing shaft columns for a building: spread x positions inside a
/// `gw`-wide grid, identical on every floor (no floor fold) so wet rooms
/// stack vertically. Capped at 4 shafts; empty when `count` is 0 or the
/// grid is degenerate.
pub(crate) fn wet_shaft_columns(x_id: i64, y_id: i64, seed: u64, gw: usize, count: u8) -> Vec<u8> {
    let n = usize::from(count.min(4));
    if n == 0 || gw < 3 {
        return Vec::new();
    }
    let inner = gw - 2;
    (0..n)
        .map(|i| {
            let spread = 1 + (i + 1) * inner / (n + 1);
            let jitter = (hash_coords(x_id, y_id.wrapping_add(i as i64), seed, domain::LAYOUT_WET)
                % 3) as i64
                - 1;
            (spread as i64 + jitter).clamp(1, inner as i64) as u8
        })
        .collect()
}

/// Split a region into apartment/office units with guillotine cuts.
///
/// Target count is `1 + hash % max_units` (one unit minimum); each cut
/// splits the largest splittable rect along its long axis (ties split x)
/// at a hashed position keeping ≥3 cells per side. Rects too small to split
/// stay whole, so cramped floors degrade to fewer, larger units rather than
/// slivers. Deterministic in `(base, region)`.
///
/// When `shafts` is non-empty, vertical (x) cuts additionally require both
/// halves to contain a shaft column, so no unit is ever orphaned from the
/// plumbing stacks; horizontal cuts preserve the x-span and are always
/// valid. If no valid cut exists the loop stops early.
fn split_units(region: PlaceRegion, max_units: u8, shafts: &[u8], base: u64) -> Vec<PlaceRegion> {
    if max_units <= 1 {
        return vec![region];
    }
    let contains_shaft = |x0: u8, w: u8| -> bool {
        shafts.is_empty()
            || shafts
                .iter()
                .any(|sc| *sc >= x0 && (*sc as u16) < u16::from(x0) + u16::from(w))
    };
    let target = 1 + pick(base, 0, usize::from(max_units));
    let mut rects = vec![region];
    let mut draw = 1usize;
    while rects.len() < target {
        // Largest splittable rect first (ties: lowest index), trying the
        // long axis first and the other axis as fallback.
        let mut order: Vec<usize> = (0..rects.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(u32::from(rects[i].w) * u32::from(rects[i].d)));
        let mut applied = false;
        for i in order {
            let r = rects[i];
            if r.w < 7 && r.d < 7 {
                continue;
            }
            let long_vertical = u16::from(r.w) >= u16::from(r.d);
            let first_vertical = if u16::from(r.w) == u16::from(r.d) {
                pick(base, draw, 2) == 0
            } else {
                long_vertical
            };
            draw += 1;
            for vertical in [first_vertical, !first_vertical] {
                let len = if vertical { r.w } else { r.d } as usize;
                if len < 7 {
                    continue;
                }
                let pos = 3 + pick(base, draw, len - 5);
                draw += 1;
                let (a, b) = if vertical {
                    (
                        PlaceRegion { w: pos as u8, ..r },
                        PlaceRegion {
                            x0: r.x0 + pos as u8,
                            w: r.w - pos as u8,
                            ..r
                        },
                    )
                } else {
                    (
                        PlaceRegion { d: pos as u8, ..r },
                        PlaceRegion {
                            z0: r.z0 + pos as u8,
                            d: r.d - pos as u8,
                            ..r
                        },
                    )
                };
                // Vertical cuts must not orphan a half from every shaft.
                if vertical && !(contains_shaft(a.x0, a.w) && contains_shaft(b.x0, b.w)) {
                    continue;
                }
                rects[i] = a;
                rects.push(b);
                applied = true;
                break;
            }
            if applied {
                break;
            }
        }
        if !applied {
            break;
        }
    }
    rects
}

/// Punch a unit's front door: the first `Void` cell on the unit rect's edge
/// whose outward neighbour is open (Void/Corridor/Door/Core — future
/// circulation, never a wall to the outside), in a hashed rotation so doors
/// vary. Returns whether a door was placed; packed units may have none, in
/// which case their room doors still open onto circulation.
fn punch_unit_door(g: &mut Floor, unit: PlaceRegion, base: u64) -> bool {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let (ux0, uz0, uw, ud) = (
        usize::from(unit.x0),
        usize::from(unit.z0),
        usize::from(unit.w),
        usize::from(unit.d),
    );
    if uw == 0 || ud == 0 {
        return false;
    }
    // Edge cells just inside the rect boundary, clockwise from the top.
    let mut edge: Vec<(usize, usize, usize, usize)> = Vec::new();
    for x in ux0..ux0 + uw {
        edge.push((x, uz0, x, uz0.saturating_sub(1)));
        edge.push((x, uz0 + ud - 1, x, uz0 + ud));
    }
    for z in uz0 + 1..uz0 + ud.saturating_sub(1) {
        edge.push((ux0, z, ux0.saturating_sub(1), z));
        edge.push((ux0 + uw - 1, z, ux0 + uw, z));
    }
    if edge.is_empty() {
        return false;
    }
    let rot = pick(base, 0, edge.len());
    edge.rotate_left(rot);
    for (x, z, nx, nz) in edge {
        if x == 0 || z == 0 || x + 1 >= gw || z + 1 >= gd {
            continue;
        }
        if g.tiles[z * gw + x] != Tile::Void {
            continue;
        }
        if nx >= gw || nz >= gd {
            continue;
        }
        match g.tiles[nz * gw + nx] {
            Tile::Void | Tile::Corridor | Tile::Door | Tile::Core => {
                g.tiles[z * gw + x] = Tile::Door;
                g.kinds[z * gw + x] = 0;
                return true;
            }
            Tile::Wall | Tile::Room => {}
        }
    }
    false
}

/// Generate the `floor`-th storey of an interior with a `role`.
///
/// See [`generate_layout`] for the full carving pipeline. `x_id`/`y_id` are the
/// interior id's coordinate halves and `seed` the world seed; room and door
/// draws mix in the floor number so storeys vary, while `core` (size,
/// position) arrives precomputed — stacked once per building, or wandering
/// per floor when the blueprint opts in.
#[must_use]
#[allow(clippy::too_many_arguments)] // one bundle per pipeline stage; splitting hides the flow
fn generate_floor(
    x_id: i64,
    y_id: i64,
    seed: u64,
    floor: u8,
    role: crate::layout::FloorRole,
    core: (u8, u8, u8),
    ctx: &InteriorContext,
    blueprint: &Blueprint,
    shafts: &[u8],
) -> Floor {
    use crate::layout::FloorRole;
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

    // Vertical circulation shaft at its precomputed rect (stacked across
    // floors, or wandering when the blueprint opts in).
    let (core_size, core_x, core_z) = core;
    paint_core(&mut g, core_x, core_z, core_size);

    match role {
        // Street level: entrance door on the lot edge; the reserved cell
        // inside stays corridor so arrivals connect. The lobby halo paints
        // after rooms (below) so it can never starve small floors.
        FloorRole::Ground => {
            let entrance_base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_DOOR);
            punch_entrance_door(&mut g, ctx.door_side, entrance_base);
            reserve_entrance(&mut g, ctx.door_side, entrance_base);
        }
        // Upper floors open onto the shaft, never onto the street.
        FloorRole::Typical | FloorRole::Top => {
            punch_core_door(&mut g, core_x, core_z, core_size);
        }
    }

    // Placeable region: the whole interior, or north of a single-loaded
    // corridor band painted along the south interior edge (rooms line one
    // side, circulation the other).
    let region = if blueprint.corridor == 1 && gd >= 6 {
        for x in 1..gw - 1 {
            let idx = (gd - 2) * gw + x;
            if g.tiles[idx] == Tile::Void {
                g.tiles[idx] = Tile::Corridor;
            }
        }
        PlaceRegion {
            x0: 1,
            z0: 1,
            w: (gw - 2) as u8,
            d: (gd - 4) as u8,
        }
    } else {
        PlaceRegion::interior(gw, gd)
    };

    // Weighted room placement against the free area, per unit region.
    let rooms = blueprint.room_slice();
    let mut placed: Vec<PlacedRoom> = Vec::new();
    if !rooms.is_empty() && region.w > 0 && region.d > 0 {
        let room_base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_ROOM);
        let smallest = smallest_template(rooms);

        // Unit subdivision (open plan when unit_max is 0): each unit gets its
        // own anchor order, rooms, guarantee, and front door. Cuts keep every
        // unit on a shaft column so wet rooms can always stack.
        let units = split_units(
            region,
            blueprint.unit_max,
            shafts,
            floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_ROOM),
        );

        // Pass A — minimums: each template with min_count>0 is placed up to
        // its minimum across the units (round-robin), wet rooms through the
        // shaft-aware path. Best-effort per template: a template that fits
        // nowhere is skipped, never retried forever.
        let mut needs: Vec<(&BlueprintRoom, u8)> = rooms
            .iter()
            .map(|r| (r, r.min_count))
            .filter(|(_, n)| *n > 0)
            .collect();
        let mut draw_k = 10_000usize;
        for unit in &units {
            let anchors = shuffled_anchors(&g, *unit, room_base, 0);
            for (room, need) in needs.iter_mut().map(|(r, n)| (*r, n)) {
                while *need > 0 {
                    let mut done = false;
                    for (ax, az) in &anchors {
                        let idx = usize::from(*az) * gw + usize::from(*ax);
                        if g.tiles[idx] != Tile::Void {
                            continue;
                        }
                        if let Some((x0, z0, w, d)) = attempt_place(
                            &mut g, *ax, *az, room, smallest, shafts, *unit, room_base, draw_k,
                            true,
                        ) {
                            draw_k += 1;
                            paint_room(&mut g, x0, z0, w, d, room.kind);
                            placed.push(PlacedRoom {
                                x0,
                                z0,
                                w,
                                d,
                                doors: room.door_count(),
                            });
                            done = true;
                            break;
                        }
                    }
                    if done {
                        *need -= 1;
                    } else {
                        break;
                    }
                }
            }
        }

        // Pass B — fill: weighted rolls per anchor inside each unit.
        for (ui, unit) in units.iter().enumerate() {
            let anchors = shuffled_anchors(&g, *unit, room_base, ui);
            for (k, (ax, az)) in anchors.into_iter().enumerate() {
                // Bound the work on very large footprints; 128 fills any sane home.
                if k >= 128 {
                    break;
                }
                let idx = usize::from(az) * gw + usize::from(ax);
                if g.tiles[idx] != Tile::Void {
                    continue;
                }
                let kk = ui * 4096 + k;
                let roll = unit_draw(room_base, kk);
                let room = roll_room(rooms, roll);
                if let Some((x0, z0, w, d)) = attempt_place(
                    &mut g, ax, az, room, smallest, shafts, *unit, room_base, kk, false,
                ) {
                    paint_room(&mut g, x0, z0, w, d, room.kind);
                    placed.push(PlacedRoom {
                        x0,
                        z0,
                        w,
                        d,
                        doors: room.door_count(),
                    });
                }
            }

            // Unit guarantee: a unit left room-less gets the smallest fitting
            // template forced in, so no apartment degrades to bare corridor.
            // Prefer a non-wet template here so the guarantee never breaks
            // shaft alignment (minimums already covered wet rooms globally).
            if !placed
                .iter()
                .any(|p| unit.contains(i64::from(p.x0), i64::from(p.z0), 1, 1))
            {
                let forced = rooms
                    .iter()
                    .filter(|r| r.tags & TAG_WET == 0)
                    .min_by_key(|r| u16::from(r.min_w) * u16::from(r.min_d))
                    .or(smallest);
                if let Some(forced) = forced {
                    'force: for dz in 0..usize::from(unit.d) {
                        for dx in 0..usize::from(unit.w) {
                            let (ax, az) = (unit.x0 as usize + dx, unit.z0 as usize + dz);
                            if ax >= gw || az >= gd {
                                continue;
                            }
                            let idx = az * gw + ax;
                            if g.tiles[idx] != Tile::Void {
                                continue;
                            }
                            if let Some((x0, z0, w, d)) = try_place_room(
                                &mut g, ax as u8, az as u8, forced, *unit, room_base, 999_983,
                            ) {
                                paint_room(&mut g, x0, z0, w, d, forced.kind);
                                placed.push(PlacedRoom {
                                    x0,
                                    z0,
                                    w,
                                    d,
                                    doors: forced.door_count(),
                                });
                                break 'force;
                            }
                        }
                    }
                }
            }

            // Unit front door onto circulation.
            punch_unit_door(&mut g, *unit, room_base.wrapping_add(ui as u64));
        }

        // A door from each room onto circulation (door 0 draws at k = 1,
        // independently of the entrance pick at k = 0 on the same stream).
        let door_base = floor_hash(x_id, y_id, seed, floor, domain::LAYOUT_DOOR);
        for p in &placed {
            for di in 0..p.doors {
                room_door(&mut g, (p.x0, p.z0, p.w, p.d), door_base, di as usize);
            }
        }
    }

    // Ground-floor lobby halo, painted after rooms so leftover space becomes
    // open circulation without ever stealing room cells on small floors.
    if role == FloorRole::Ground {
        paint_lobby_halo(&mut g, core_x, core_z, core_size);
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

/// Whether a `w×d` room with top-left corner `(x0, z0)` fits inside `region`:
/// the rectangle itself must be free `Void`, and every cell in a one-tile
/// margin around it must not be an already-placed room. The margin is what
/// becomes the corridor channel keeping each room reachable. Exterior `Wall`
/// (rooms may hug the block perimeter), the circulation `Core` (a room can
/// open straight onto it), and an entrance `Door` are all valid margin
/// neighbours.
fn room_fits(g: &Floor, x0: i64, z0: i64, w: u8, d: u8, region: PlaceRegion) -> bool {
    if !region.contains(x0, z0, w, d) {
        return false;
    }
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

/// Try to place `room` with top-left at `(ax, az)` inside `region`, rolling
/// a size within the template's bounds and falling back to the closest fit.
/// On success paints the room and returns its rect `(x0, z0, w, d)`.
fn try_place_room(
    g: &mut Floor,
    ax: u8,
    az: u8,
    room: &BlueprintRoom,
    region: PlaceRegion,
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
        if room_fits(g, i64::from(ax), i64::from(az), w, d, region) {
            return Some((ax, az, w, d));
        }
    }
    None
}

/// Smallest template by footprint area (the cramped-lot fallback).
fn smallest_template(rooms: &[BlueprintRoom]) -> Option<&BlueprintRoom> {
    rooms
        .iter()
        .min_by_key(|r| u16::from(r.min_w) * u16::from(r.min_d))
}

/// Candidate anchors inside `region`: every free cell in a deterministic
/// pseudo-random order (Fisher–Yates on the hash stream, offset by unit so
/// units shuffle differently).
fn shuffled_anchors(g: &Floor, region: PlaceRegion, base: u64, unit_idx: usize) -> Vec<(u8, u8)> {
    let gw = usize::from(g.width);
    let gd = usize::from(g.depth);
    let mut anchors: Vec<(u8, u8)> = Vec::new();
    for dz in 0..usize::from(region.d) {
        for dx in 0..usize::from(region.w) {
            let (x, z) = (region.x0 as usize + dx, region.z0 as usize + dz);
            if x < gw && z < gd && g.tiles[z * gw + x] == Tile::Void {
                anchors.push((x as u8, z as u8));
            }
        }
    }
    let off = unit_idx * 4096;
    for i in (1..anchors.len()).rev() {
        let j = pick(base, i + off, i + 1);
        anchors.swap(i, j);
    }
    anchors
}

/// Attempt one room placement: shaft-aware for WET rooms, then the ordinary
/// anchor attempt with a smallest-template fallback (cramped lots degrade to
/// small rooms instead of corridor-only shells).
///
/// `allow_offshaft` governs WET rooms whose shafts are fully blocked:
/// minimum passes set it (a required kitchen matters more than its stack),
/// fill passes clear it (a rolled wet room that fits no shaft is skipped so
/// every placed wet room provably stacks).
#[allow(clippy::too_many_arguments)] // one bundle per placement input; splitting hides the flow
fn attempt_place(
    g: &mut Floor,
    ax: u8,
    az: u8,
    room: &BlueprintRoom,
    fallback: Option<&BlueprintRoom>,
    shafts: &[u8],
    region: PlaceRegion,
    base: u64,
    k: usize,
    allow_offshaft: bool,
) -> Option<(u8, u8, u8, u8)> {
    if room.tags & TAG_WET != 0 && !shafts.is_empty() {
        if let Some(rect) = try_place_wet(g, az, room, shafts, region, base, k) {
            return Some(rect);
        }
        if !allow_offshaft {
            return None;
        }
        // Shafts full is not the lot's failure: fall through to normal.
    }
    let mut rect = try_place_room(g, ax, az, room, region, base, k);
    if rect.is_none() {
        if let Some(s) = fallback {
            if !std::ptr::eq(room, s) {
                rect = try_place_room(g, ax, az, s, region, base, k);
            }
        }
    }
    rect
}

/// Place a WET room covering a plumbing shaft: roll a size, then try every
/// shaft-covering x origin at the anchor's row (nearest shaft first is
/// implied by shaft order). Guarantees wet rooms sit on a stack whenever
/// the shafts have room, keeping plumbing vertical across floors.
fn try_place_wet(
    g: &mut Floor,
    az: u8,
    room: &BlueprintRoom,
    shafts: &[u8],
    region: PlaceRegion,
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
    let (rx0, rz0, rw, rd) = (
        i64::from(region.x0),
        i64::from(region.z0),
        i64::from(region.w),
        i64::from(region.d),
    );
    if rd <= 0 || rw <= 0 {
        return None;
    }
    // Anchor row first, then the rest of the region in rotation: keeps wet
    // rooms near their shuffled anchor while covering the stack.
    let azr = (i64::from(az) - rz0).clamp(0, rd - 1);
    for (w, d) in combos {
        for sc in shafts {
            // Origins whose rect covers the shaft column, clamped to region.
            let lo = (*sc as i64 - i64::from(w) + 1).max(rx0);
            let hi = (*sc as i64).min(rx0 + rw - i64::from(w));
            if lo > hi {
                continue;
            }
            for t in 0..rd {
                let z0 = rz0 + (azr + t) % rd;
                for x0 in lo..=hi {
                    if room_fits(g, x0, z0, w, d, region) {
                        return Some((x0 as u8, z0 as u8, w, d));
                    }
                }
            }
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

/// Punch `door_idx`-th `Door` on a room's boundary: walk the room's perimeter
/// in a deterministic rotation of the hash stream (offset per door index so
/// multi-door rooms open at distinct spots) and turn the first margin cell
/// the room faces into a `Door` (the margin is pure circulation by
/// construction). The door lives in the corridor channel, so the room itself
/// stays intact, the opening always leads to circulation, and nested rooms
/// of any size remain fully walled.
fn room_door(g: &mut Floor, rect: (u8, u8, u8, u8), base: u64, door_idx: usize) {
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
    // Door 0 draws at k = 1 (independent of the entrance pick at k = 0);
    // further doors rotate from later draws so multi-door rooms spread out.
    let rot = pick(base, 1 + door_idx * 2, perimeter.len());
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
            false,
            crate::layout::BuildingRole::Ordinary,
            ZoneType::Commercial,
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
            true,
            crate::layout::BuildingRole::Landmark,
            ZoneType::Commercial,
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
            false,
            crate::layout::BuildingRole::Ordinary,
            ZoneType::Commercial,
        )
    }

    /// Core tile coordinates of a floor, sorted, for cross-floor comparison.
    fn core_coords(f: &crate::layout::Floor) -> Vec<(u8, u8)> {
        let mut out = Vec::new();
        for z in 0..f.depth {
            for x in 0..f.width {
                if f.tiles[f.index(x, z)] == Tile::Core {
                    out.push((x, z));
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// A three-storey apartment block fixture (Ground + Typical + Top) on a
    /// roomy 14×12 lot: units subdivide, minimums bind, shafts have space.
    fn apartments_ctx(id: InteriorId, seed: u64) -> InteriorContext {
        InteriorContext::new(
            id,
            ZoneType::Residential,
            [0.0; 5],
            12.0,
            4.0,
            64,
            14,
            12,
            1,
            DoorSide::South,
            seed,
            false,
            crate::layout::BuildingRole::Ordinary,
            ZoneType::Commercial,
        )
    }

    /// Room tiles grouped per unit rect of a subdivided floor.
    fn rooms_per_unit(f: &crate::layout::Floor, units: &[PlaceRegion]) -> Vec<usize> {
        units
            .iter()
            .map(|u| {
                let mut n = 0;
                for dz in 0..usize::from(u.d) {
                    for dx in 0..usize::from(u.w) {
                        let (x, z) = (u.x0 as usize + dx, u.z0 as usize + dz);
                        if x < f.width as usize
                            && z < f.depth as usize
                            && f.tiles[f.index(x as u8, z as u8)] == Tile::Room
                        {
                            n += 1;
                        }
                    }
                }
                n
            })
            .collect()
    }

    /// Doors on the outer wall ring (street entrances, never room/core doors,
    /// which live strictly inside the ring).
    fn ring_doors(f: &crate::layout::Floor) -> usize {
        let (w, d) = (f.width, f.depth);
        let mut n = 0;
        for x in 0..w {
            if f.tiles[f.index(x, 0)] == Tile::Door {
                n += 1;
            }
            if f.tiles[f.index(x, d - 1)] == Tile::Door {
                n += 1;
            }
        }
        for z in 1..d.saturating_sub(1) {
            if f.tiles[f.index(0, z)] == Tile::Door {
                n += 1;
            }
            if f.tiles[f.index(w - 1, z)] == Tile::Door {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn core_stacks_vertically_across_floors() {
        // One shaft position per building: every storey carries the same core
        // tiles (default blueprints stack; wandering is opt-in).
        let ctx = tower_ctx(7, 42);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let layout = generate_layout(7, &ctx, &bp);
        assert!(layout.floors.len() > 3);
        let first = core_coords(&layout.floors[1]);
        assert!(!first.is_empty());
        for f in layout.floors.iter().skip(2).take(layout.floors.len() - 3) {
            assert_eq!(core_coords(f), first, "shaft wanders between storeys");
        }
    }

    #[test]
    fn typical_floors_repeat_one_layout() {
        // Middle storeys clone a single typical program; ground and top keep
        // their own roles.
        let ctx = tower_ctx(7, 42);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let layout = generate_layout(7, &ctx, &bp);
        let n = layout.floors.len();
        assert!(n > 3);
        for f in &layout.floors[2..n - 1] {
            assert_eq!(f, &layout.floors[1], "typical floor not repeated");
        }
        assert_ne!(
            layout.floors[0], layout.floors[1],
            "ground should differ from typical (entrance + lobby)"
        );
    }

    #[test]
    fn entrance_door_lives_on_the_ground_floor_only() {
        // Exactly one ring door on floor 0; no street doors above (upper
        // floors open onto the core instead).
        let ctx = tower_ctx(7, 42);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let layout = generate_layout(7, &ctx, &bp);
        assert_eq!(ring_doors(&layout.floors[0]), 1);
        for (i, f) in layout.floors.iter().enumerate().skip(1) {
            assert_eq!(ring_doors(f), 0, "street door on floor {i}");
        }
    }

    #[test]
    fn top_floor_expands_the_mechanical_core() {
        let ctx = tower_ctx(7, 42);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let layout = generate_layout(7, &ctx, &bp);
        let n = layout.floors.len();
        let typical_core = core_coords(&layout.floors[1]).len();
        let top_core = core_coords(&layout.floors[n - 1]).len();
        assert!(top_core >= typical_core, "top core did not expand");
    }

    #[test]
    fn vary_typical_opt_in_rerolls_storeys() {
        let ctx = tower_ctx(7, 42);
        let mut bp = crate::layout::blueprint_defaults(ctx.zone);
        bp.vary_typical = 1;
        let layout = generate_layout(7, &ctx, &bp);
        assert!(layout.floors.len() > 3);
        assert_ne!(
            layout.floors[1], layout.floors[2],
            "vary_typical should re-roll middle storeys"
        );
    }

    #[test]
    fn wandering_core_opt_in_moves_the_shaft() {
        let ctx = tower_ctx(11, 99);
        let mut bp = crate::layout::blueprint_defaults(ctx.zone);
        bp.wandering_core = 1;
        let layout = generate_layout(11, &ctx, &bp);
        let a = core_coords(&layout.floors[0]);
        let b = core_coords(&layout.floors[1]);
        assert!(!a.is_empty() && !b.is_empty());
        assert_ne!(a, b, "wandering core should move between storeys");
    }

    #[test]
    fn units_partition_the_floor_and_hold_rooms() {
        // The typical floor subdivides into 1–3 unit rects that tile the
        // region, and the unit guarantee leaves no apartment room-less.
        let ctx = apartments_ctx(21, 7);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        assert_eq!(bp.unit_max, 3);
        let layout = generate_layout(21, &ctx, &bp);
        let floor = &layout.floors[1]; // Typical
        let region = PlaceRegion::interior(14, 12);
        // Recompute the split exactly as generation does (same inputs:
        // split_id(21) = (21, 0), seed 7, floor 1, same shafts).
        let shafts = wet_shaft_columns(21, 0, 7, 14, bp.wet_shafts);
        let units = split_units(
            region,
            bp.unit_max,
            &shafts,
            floor_hash(21, 0, 7, 1, domain::LAYOUT_ROOM),
        );
        assert!((1..=3).contains(&units.len()));
        // Partition check: area preserved, no overlaps.
        let area: u32 = units.iter().map(|u| u32::from(u.w) * u32::from(u.d)).sum();
        assert_eq!(area, u32::from(region.w) * u32::from(region.d));
        let counts = rooms_per_unit(floor, &units);
        for (i, n) in counts.iter().enumerate() {
            assert!(*n > 0, "unit {i} holds no rooms");
        }
    }

    #[test]
    fn minimums_hold_kitchen_and_bath_on_homes() {
        // Residential minimums (kitchen kind 21, bath kind 23) bind on every
        // storey of a roomy home.
        let ctx = apartments_ctx(22, 8);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let layout = generate_layout(22, &ctx, &bp);
        for (i, f) in layout.floors.iter().enumerate() {
            assert!(f.kinds.contains(&21), "floor {i} missing kitchen");
            assert!(f.kinds.contains(&23), "floor {i} missing bath");
        }
    }

    #[test]
    fn wet_rooms_stack_on_shaft_columns() {
        // Every wet-kind tile (kitchen 21, bath 23) sits within a room width
        // of a building-constant shaft column, on every floor.
        let ctx = apartments_ctx(23, 9);
        let bp = crate::layout::blueprint_defaults(ctx.zone);
        let (x_id, y_id) = split_id(23);
        let shafts = wet_shaft_columns(x_id, y_id, 9, 14, bp.wet_shafts);
        assert_eq!(shafts.len(), 2);
        let layout = generate_layout(23, &ctx, &bp);
        for (i, f) in layout.floors.iter().enumerate() {
            for z in 0..f.depth {
                for x in 0..f.width {
                    let idx = f.index(x, z);
                    if f.tiles[idx] == Tile::Room && (f.kinds[idx] == 21 || f.kinds[idx] == 23) {
                        let near = shafts.iter().any(|sc| (x as i64 - *sc as i64).abs() <= 2);
                        assert!(near, "wet tile at ({x},{z}) off-shaft on floor {i}");
                    }
                }
            }
        }
    }

    #[test]
    fn mixed_use_ground_serves_retail() {
        // A 3-storey home resolves a commercial ground override: retail kind
        // 30 below, homes above. Short homes keep a residential ground.
        // (Retail placement is rolled, so scan seeds for the fixture instead
        // of hardcoding one.)
        use crate::layout::default_blueprints;
        let table = default_blueprints();
        let bp = &table[ZoneType::Residential as usize];
        let mut picked = None;
        for seed in 10u64..40 {
            let ctx = apartments_ctx(24, seed);
            let ground = resolve_ground_override(&ctx, bp, &table);
            assert_eq!(ground.rooms[0].kind, 30);
            let layout = generate_layout_with_ground(24, &ctx, bp, ground);
            if layout.floors[0].kinds.contains(&30) {
                picked = Some((seed, layout));
                break;
            }
        }
        let (seed, layout) = picked.expect("no seed rolled retail on ground");
        assert!(
            !layout.floors[1].kinds.contains(&30),
            "retail leaked upstairs (seed {seed})"
        );
        // Two-storey homes keep a residential ground floor.
        let short = home_ctx(25, 10);
        assert_eq!(short.floor_count, 2);
        let same = resolve_ground_override(&short, bp, &table);
        assert!(std::ptr::eq(same, bp));
    }

    #[test]
    fn multi_door_rooms_open_more_than_once() {
        // One big two-door room on a clear floor punches two distinct doors.
        let mut g = Floor::empty(10, 10);
        for z in 1..9 {
            for x in 1..9 {
                g.tiles[z * 10 + x] = Tile::Corridor;
            }
        }
        paint_wall_ring(&mut g);
        paint_room(&mut g, 3, 3, 3, 3, 20);
        let base = 0x1234_5678_9abc_def0;
        room_door(&mut g, (3, 3, 3, 3), base, 0);
        room_door(&mut g, (3, 3, 3, 3), base, 1);
        let doors = g.tiles.iter().filter(|t| **t == Tile::Door).count();
        assert!(doors >= 2, "two-door room opened {doors} door(s)");
    }

    #[test]
    fn single_loaded_policy_confines_rooms_north() {
        // Corridor band along the south interior edge; rooms never touch it.
        let ctx = apartments_ctx(26, 11);
        let mut bp = crate::layout::blueprint_defaults(ctx.zone);
        bp.corridor = 1;
        let layout = generate_layout(26, &ctx, &bp);
        for (i, f) in layout.floors.iter().enumerate() {
            let (w, d) = (f.width as usize, f.depth as usize);
            for x in 1..w - 1 {
                let t = f.tiles[(d - 2) * w + x];
                // Corridor band, doors punched onto it, or the shaft itself
                // opening onto the corridor — never rooms or walls.
                assert!(
                    t == Tile::Corridor || t == Tile::Door || t == Tile::Core,
                    "band cell is {t:?} on floor {i}"
                );
            }
            for z in 0..f.depth {
                for x in 0..f.width {
                    if f.tiles[f.index(x, z)] == Tile::Room {
                        assert!(
                            z < f.depth - 2,
                            "room south of the corridor band on floor {i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn split_units_partition_deterministically() {
        let region = PlaceRegion {
            x0: 1,
            z0: 1,
            w: 12,
            d: 10,
        };
        let a = split_units(region, 3, &[], 0xabcd);
        let b = split_units(region, 3, &[], 0xabcd);
        assert_eq!(a, b);
        assert!((1..=3).contains(&a.len()));
        // Exact partition: area preserved.
        let area: u32 = a.iter().map(|u| u32::from(u.w) * u32::from(u.d)).sum();
        assert_eq!(area, 12 * 10);
        // Single-unit request stays whole.
        assert_eq!(split_units(region, 0, &[], 0xabcd), vec![region]);
        assert_eq!(split_units(region, 1, &[], 0xabcd), vec![region]);
    }

    #[test]
    fn splits_keep_every_unit_on_a_shaft() {
        // With shafts present, no vertical cut orphans a half: every unit
        // rect covers a shaft column.
        let region = PlaceRegion {
            x0: 1,
            z0: 1,
            w: 12,
            d: 10,
        };
        let shafts = [4u8, 9u8];
        for base in [0xabcd, 1, 99, 12345] {
            for max in 2..=4 {
                for u in split_units(region, max, &shafts, base) {
                    assert!(
                        shafts.iter().any(
                            |sc| *sc >= u.x0 && (*sc as u16) < u16::from(u.x0) + u16::from(u.w)
                        ),
                        "unit {u:?} orphaned from shafts"
                    );
                }
            }
        }
    }

    #[test]
    fn wet_shaft_columns_are_placed_and_stable() {
        let a = wet_shaft_columns(5, -3, 77, 14, 2);
        let b = wet_shaft_columns(5, -3, 77, 14, 2);
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        for sc in a {
            assert!((1..=12).contains(&sc));
        }
        assert!(wet_shaft_columns(5, -3, 77, 14, 0).is_empty());
        assert!(wet_shaft_columns(5, -3, 77, 14, 9).len() <= 4);
    }

    #[test]
    fn unplaceable_minimums_skip_without_panic() {
        // A minimum template larger than the grid can never fit: the floor
        // still completes (the minimum is best-effort, never a hang).
        let mut bp = crate::layout::blueprint_defaults(ZoneType::Residential);
        bp.rooms[0] = BlueprintRoom::new(20, 1.0, 30, 30, 30, 30, 5, 0, 1);
        bp.room_count = 1;
        bp.unit_max = 0;
        let ctx = lot_ctx(27, DoorSide::West, 3);
        let layout = generate_layout(27, &ctx, &bp);
        assert!(!layout.floors.is_empty());
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
            true,
            crate::layout::BuildingRole::Landmark,
            ZoneType::Commercial,
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
            false,
            crate::layout::BuildingRole::Ordinary,
            ZoneType::Residential,
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
