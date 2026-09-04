# Interior Generation & Blueprints — Urbix

This document describes how Urbix turns a built cell into a deterministic
interior mini-world and how the per-zone "blueprint" rule tables that shape room
placement are defined. It mirrors `Urbix_Project.md §4.4 / §8.1` (Milestone 9)
but dives into the actual data model as implemented in `src/layout.rs`,
`src/interior.rs`, and `src/config.rs`.

Status (Milestone 9): landed. The context/blueprint data model, the
`InteriorState::generate(id, ctx)` signature, and the full room-placement
algorithm (weighted rolls from the blueprint tables, greedy placement with a
navigable margin, corridor fill, per-floor variation, and a street-facing
entrance) are implemented and tested.

## 1. Overview

A generated interior is a **separate mini-world**, not part of the outdoor chunk
grid. It is keyed by `InteriorId` (`domain::INTERIOR` hash of the cell's world
coords, `src/building.rs`) and cached in `InteriorCache` purely by capacity
(no draw distance — `src/interior.rs:287`).

```
built cell (height > 0)
   │
   ▼
InteriorContext (zone, affinity, height → floor_count, footprint, palette, seed)
   │
   ▼
Blueprint for the dominant zone   ──►   InteriorLayout { Floor[] }
   │                                     (Tile grid per storey + room-kind tags)
   ▼
InteriorCache.get(id) / insert        renderer / consumer
```

Every step is a pure function of `(InteriorId, seed, domain)` via
`hash::hash_coords`, exactly like the exterior pipeline — same lot, same
interior, cross-chunk-consistent.

## 2. InteriorContext — the exterior→interior bridge

`src/layout.rs:83`, `#[repr(C)]` so it can cross the FFI for inspection or
override. It is a snapshot of the exterior lot the interior belongs to:

| Field | Meaning |
|---|---|---|
| `id` | stable `InteriorId` (cache key) |
| `zone` | dominant `ZoneType` (affinity argmax) — picks the blueprint family |
| `zone_affinity` | blended `[f32; ZONE_COUNT]` — lets layouts blend near fuzzy borders |
| `height` | exterior building height (world units); 0 = no building |
| `floor_count` | floors derived from `height` (≥ 1 for a built lot) |
| `footprint_w/d` | interior grid width/depth in tiles (block-derived, floored at 7×7) |
| `palette_id` | exterior facade palette (rooms tint to match) |
| `door_side` | `DoorSide` of the street-facing entrance (West/East/North/South) |
| `seed` | world seed used throughout interior derivation |

The constructor `InteriorContext::new` (`src/layout.rs:121`) is the single
place the **height → floors** rule lives:

```
floor_count = height <= 0 ? 0
            : clamp(ceil(height / interior_floor_height), 1, interior_max_floors)
```

`is_built()` (`src/layout.rs:155`) is `floor_count > 0 && footprint_w > 0 &&
footprint_d > 0`.

Production callers don't build it by hand: `chunk::interior_context_for(config,
world_x, world_z, cell)` (`src/chunk.rs:143`) reconstructs it from a `Cell`
(dominant zone, clamped block footprint) via `WorldConfig::interior_context`
(`src/config.rs:388`), and `chunk::door_side_for(world_x, world_z,
block_size)` selects the street edge the lot faces (ties toward
West/East/North/South).

## 3. Tile kinds

`src/layout.rs:172`, `#[repr(u8)]`, so a floor grid packs into a flat byte
array for FFI:

| Value | Tile | Meaning |
|---|---|---|
| 0 | `Void` | outside the building volume (treated as solid) |
| 1 | `Wall` | exterior wall; sealed footprint boundary |
| 2 | `Door` | doorway between traversable tiles |
| 3 | `Core` | vertical circulation (stairs/elevator/lobby) |
| 4 | `Corridor` | horizontal circulation |
| 5 | `Room` | generic traversable floor tile |

Off-grid reads clamp to `Void` (`Floor::tile`, `src/layout.rs:378`). Each floor
carries a parallel `kinds: Vec<u8>` room-tag array: the opaque
`BlueprintRoom::kind` is stamped on every `Room` tile, and non-room tiles keep
their default (`0`) tag (`src/layout.rs:344`). Renderers use it to map rooms
back to "living", "office", etc.

## 4. Blueprint data model

Blueprints are the per-zone rule tables. Two plain `#[repr(C)]`,
`Serialize`/`Deserialize` records so artists tune interiors via `WorldConfig`
(TOML/JSON) without recompiling and the tables cross the FFI unchanged.

**`BlueprintRoom`** (`src/layout.rs:200`) — one room template:

| Field | Meaning |
|---|---|
| `kind` | opaque room-kind tag (semantics belong to the consumer/renderer) |
| `weight` | relative selection weight when rolling a room for this zone |
| `min_w/max_w` | room width bounds in tiles (inclusive) |
| `min_d/max_d` | room depth bounds in tiles (inclusive) |

**`Blueprint`** (`src/layout.rs:242`) — one zone's whole rule table:

| Field | Meaning |
|---|---|
| `margin` | structural wall-ring thickness |
| `core_size` | width of the vertical-circulation core in tiles |
| `room_count` | number of live entries in `rooms` (`0..=MAX_BLUEPRINT_ROOMS`) |
| `rooms` | `[BlueprintRoom; MAX_BLUEPRINT_ROOMS]` fixed array; only `room_count` are live |

`rooms` is a **fixed-size** array (`MAX_BLUEPRINT_ROOMS = 8`, `src/layout.rs:55`)
because a `Blueprint` must live inside the `#[repr(C)]` `WorldConfig` and cross
the FFI — it cannot hold a heap-allocated `Vec`. The live prefix is exposed by
`room_slice()` (`src/layout.rs:256`); `is_empty()` (`src/layout.rs:262`) is true
when no templates are live.

## 5. Default tables per zone

`blueprint_defaults(zone)` (`src/layout.rs:273`) hardcodes a starting table per
zone, sized to its typical footprint (dense small rooms + large core downtown,
spacious few rooms in homes). `default_blueprints()` (`src/layout.rs:327`) builds
the `[Blueprint; ZONE_COUNT]` array that initializes
`WorldConfig.interior_blueprints`.

| Zone | margin / core | room templates (kind · weight · min/max size) |
|---|---|---|
| Downtown | 2 / 3 | lobby 10 · 3.0 · 3–6²; open office 11 · 6.0 · 3–4²; meeting 12 · 4.0 · 3–5×2–4; utility 13 · 3.0 · 2–3² |
| Residential | 1 / 2 | living 20 · 4.0 · 3–5²; kitchen 21 · 3.0 · 2–3²; bedroom 22 · 4.0 · 3–4²; bathroom 23 · 1.0 · 1–2² |
| Commercial | 1 / 2 | retail 30 · 3.0 · 4–6×3–5; office/flex 31 · 3.0 · 3–5²; stockroom 32 · 2.0 · 2–3² |
| Industrial | 1 / 2 | work bay 40 · 5.0 · 4–7×3–6; office/reception 41 · 2.0 · 2–3²; washroom 42 · 1.0 · 1–2² |
| Park | 1 / 1 | shed 50 · 1.0 · 2–3² |

## 6. Configuration & override chain

`WorldConfig.interior_blueprints: [Blueprint; ZONE_COUNT]`
(`src/config.rs:119`) holds the tunable copy; it is
`#[serde(default = "default_interior_blueprints")]` (`src/config.rs:48`) so
config files written before M9 keep parsing and reproduce the default tables.

- `WorldConfig::blueprint_for(zone)` (`src/config.rs:371`) returns the tuned
  table, falling back to `blueprint_defaults(zone)` whenever a zone's table is
  empty — a valid table is always guaranteed.
- `WorldConfig::is_valid()` (`src/config.rs:251`) rejects flows that would make
  the generator panic or produce nonsense: `interior_floor_height` outside
  `1e-6..=1000.0`, `interior_max_floors == 0`, `core_size == 0`,
  `room_count > MAX_BLUEPRINT_ROOMS`, `weight < 0`, `min_w/min_d == 0`, or
  inverted `max_w < min_w` / `max_d < min_d`.
- The two scalars `interior_floor_height` (default `4.0`, `src/layout.rs:60`) and
  `interior_max_floors` (default `64`, `src/layout.rs:64`) tune the height→floors
  derivation; `WorldConfig::interior_context` feeds them to
  `InteriorContext::new`.

Overriding a blueprint in TOML (note: serde deserializes the **fixed** `rooms`
array, so all 8 slots must be present when a blueprint is specified):

```toml
[[interior_blueprints]]          # index 0 = Downtown (ZoneType order)
margin = 2
core_size = 3
room_count = 4
[[interior_blueprints.rooms]]
kind = 10; weight = 3.0; min_w = 3; max_w = 6; min_d = 3; max_d = 6
[[interior_blueprints.rooms]]
kind = 11; weight = 6.0; min_w = 3; max_w = 4; min_d = 3; max_d = 4
# ... 6 more [[interior_blueprints.rooms]] to fill the fixed array
```

See `urbix.toml.example` / `urbix.json.example` for the scalar knobs; the tables
are opt-in.

## 7. How blueprints are consumed

`generate_layout(id, ctx, blueprint)` (`src/interior.rs:187`) is the Milestone-9
generator: one `Floor` per storey, each carved by `generate_floor`
(`src/interior.rs:216`) as a pure function of `(id, floor, seed, domain)`.
Every hash stream is a distinct domain (see §8), so storeys vary independently
while remaining bit-identical for the same lot.

Per floor, in order:

1. **Sealed wall ring** (`src/interior.rs:316`): the footprint edge is exterior
   `Wall`, so nothing leaks out. A footprint narrower than 3 tiles is sealed
   solid instead (still safe for renderers).
2. **Circulation core** (`src/interior.rs:568`): a `blueprint.core_size`
   square of `Core` (stairs/elevator), position derived from
   `domain::LAYOUT_FLOOR`. All floors of a storey have one; it never touches
   the wall ring. The core is capped to about half the interior so a small
   lot keeps room for at least one room plus circulation (small-zone lots
   spawn as 6×6 grids, never as zero-room stubs).
3. **Street entrance** (`src/interior.rs:335`): a `Door` on the wall ring
   facing `ctx.door_side`, at a hashed offset along that edge. The cell just
   inside it is reserved as `Corridor` (`src/interior.rs:360`) so the entrance
   always opens into the interior.
4. **Weighted room placement** (`src/interior.rs:257`): candidate anchors are
   every free cell, visited in a Fisher–Yates order from the `LAYOUT_ROOM`
   stream. At each anchor a template is rolled against `weight` via
   `roll_room` (`src/interior.rs:550`) and a size within its `min`/`max`
   bounds is drawn; `try_place_room` (`src/interior.rs:421`) then walks the
   size candidates closest to the roll and places the first that fits.
`room_fits` (`src/interior.rs:391`) requires the rectangle to be free and
    its one-tile margin to contain no other room — the margin, plus the fact
    rooms may hug walls and the core, is what keeps every room reachable.
    Rooms are painted with their opaque `kind` tag (`src/interior.rs:457`).
    When the rolled template cannot fit the remaining free area (typical on
    small lots), placement retries once with the blueprint's smallest template,
    so no lot degrades to a corridor-only shell.
5. **Doors** (`src/interior.rs:474`): the `LAYOUT_DOOR` stream rotates each
   room's perimeter (k=1 onward, after the entrance at k=0) and turns the
   first facing margin cell into a `Door`, so every room has exactly one
   opening into circulation.
6. **Corridor fill**: every leftover free cell becomes `Corridor`, so the
   margin channels connect all rooms, the core, and the street entrance into
   one navigable interior.

The result is deterministic, sealed (only entrance/room `Door`s break the
ring), and every room opens onto circulation — covered by the unit tests in
`src/interior.rs` (`rooms_are_placed_walled_sealed_and_reachable`,
`interiors_vary_across_floors_and_seeds`, `entrance_door_faces_the_context_side`,
`degenerate_footprint_is_sealed`).

## 8. Hash domains

Reserved for interior work (see `src/hash.rs:62` and `include/urbix.h`):

| Domain | Constant | Use |
|---|---|---|
| 40 | `LAYOUT_PICK` | blueprint selection (reserved) |
| 41 | `LAYOUT_FLOOR` | core placement; per-floor variation |
| 42 | `LAYOUT_ROOM` | room-kind rolls, anchor shuffle, size draws |
| 43 | `LAYOUT_ROOM_SIZE` | size draws (reserved; sizes draw on the room stream) |
| 44 | `LAYOUT_DOOR` | entrance pick (k=0) + room doors (k≥1) |
| 45 | `LAYOUT_FURNITURE` | slot density (reserved for a later milestone) |

## Reference

- Data model: `src/layout.rs` (context, tiles, floors, blueprints, defaults).
- Generator + cache: `src/interior.rs` (`InteriorState`, `generate_layout`,
  `InteriorCache`).
- Config wiring: `src/config.rs` (`interior_floor_height`, `interior_max_floors`,
  `interior_blueprints`, `blueprint_for`, `interior_context`, `is_valid`).
- Cell → context: `src/chunk.rs:139` (`interior_context_for`).
- Default `interior_id`: `src/building.rs`, `domain::INTERIOR`.
- Headers: `docs/api.md`, `Urbix_Project.md §4.4 / §8.1`.