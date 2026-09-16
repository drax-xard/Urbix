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

Status (Milestone 10): landed. The generated interior crosses the C border via
`UrbixInterior` / `urbix_generate_interior` / `urbix_interior_free` (see §9),
so renderers consuming chunks can also render room layouts with no Rust.

Status (Milestone 13): landed. Interiors are vertically structured: lot-true
contexts (pack-rect footprints, corner/role/secondary-zone), floor roles
(Ground/Typical×N/Top with typical repetition), one stacked circulation
shaft per building, and a ground-only street entrance with lobby halo and
core lobby doors above. See the Roadmap below for M14–M15.

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
| `footprint_w/d` | lot pack-rect width/depth in tiles (`lot::lot_rect`, clamped 3–64) |
| `palette_id` | exterior facade palette (rooms tint to match) |
| `door_side` | `DoorSide` of the street-facing entrance (West/East/North/South) |
| `seed` | world seed used throughout interior derivation |
| `corner` | lot touches streets on two axes (corner-tower treatment) |
| `frontage_depth` | lot depth perpendicular to the entrance edge (derived) |
| `building_role` | `BuildingRole` (Ordinary/Landmark/Market/TowerPark) |
| `secondary_zone` | affinity runner-up (mixed-use ground floors) |

The constructor `InteriorContext::new` (`src/layout.rs`) is the single
place the **height → floors** rule lives:

```
floor_count = height <= 0 ? 0
            : clamp(ceil(height / interior_floor_height), 1, interior_max_floors)
```

`is_built()` (`src/layout.rs:155`) is `floor_count > 0 && footprint_w > 0 &&
footprint_d > 0`.

Production callers don't build it by hand: `chunk::interior_context_for(config,
voronoi, world_x, world_z, cell)` (`src/chunk.rs`) reconstructs it from a
`Cell` — dominant zone, lot pack rect, corner, building role (block program /
landmark recomputed pure from world coords), secondary zone — via
`WorldConfig::interior_context` (`src/config.rs`), and
`chunk::door_side_for(world_x, world_z, block_size)` selects the street edge
the lot faces (ties toward West/East/North/South).

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
| `min_count` | minimum placements per floor (best-effort; skipped if unplaceable) |
| `tags` | adjacency bits: 1 WET (shafts) · 2 QUIET · 4 PUBLIC · 8 STREET |
| `doors` | doors punched per room (`0` reads as 1) |

**`Blueprint`** (`src/layout.rs`) — one zone's whole rule table:

| Field | Meaning |
|---|---|
| `margin` | structural wall-ring thickness |
| `core_size` | width of the vertical-circulation core in tiles |
| `room_count` | number of live entries in `rooms` (`0..=MAX_BLUEPRINT_ROOMS`) |
| `rooms` | `[BlueprintRoom; MAX_BLUEPRINT_ROOMS]` fixed array; only `room_count` are live |
| `vary_typical` | nonzero re-rolls typical floors per storey (`0` = generate once, clone) |
| `wandering_core` | nonzero re-hashes the core per floor (`0` = one stacked shaft) |
| `unit_max` | max apartment units per floor (`0` = open plan) |
| `wet_shafts` | plumbing shaft columns per building (`0` = off, capped at 4) |
| `ground_zone` | ground-floor blueprint as `ZoneType` index (`255` = none; retail base) |
| `corridor` | `0` double-loaded fill · `1` single-loaded (rooms north of south band) |

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
| Residential | 1 / 2 | living 20 · 4.0 · 3–5²; kitchen 21 · 3.0 · 2–3² (min 1, wet); bedroom 22 · 4.0 · 3–4² (quiet); bathroom 23 · 1.0 · 1–2² (min 1, wet) |
| Commercial | 1 / 2 | retail 30 · 3.0 · 4–6×3–5; office/flex 31 · 3.0 · 3–5²; stockroom 32 · 2.0 · 2–3² |
| Industrial | 1 / 2 | work bay 40 · 5.0 · 4–7×3–6; office/reception 41 · 2.0 · 2–3²; washroom 42 · 1.0 · 1–2² |
| Park | 1 / 1 | shed 50 · 1.0 · 2–3² |

Unit/shaft/ground policy defaults: homes split into ≤3 apartments over 2
wet shafts with a Commercial retail base on 3+ storey buildings; workplaces
stay open plan with 1 shaft; sheds stay simple (no units, no shafts).

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

`generate_layout(id, ctx, blueprint)` (`src/interior.rs`) is the generator:
storeys carry roles (`FloorRole`: floor 0 is Ground, the last is Top, the
middle are Typical), each carved by `generate_floor` as a pure function of
`(id, floor, seed, domain)`. Typical floors generate once and clone (one
repeated layout, unless the blueprint sets `vary_typical`); every hash
stream is a distinct domain (see §8), so buildings stay bit-identical for
the same lot.

Per floor, in order:

1. **Sealed wall ring**: the footprint edge is exterior
   `Wall`, so nothing leaks out. A footprint narrower than 3 tiles is sealed
   solid instead (still safe for renderers).
2. **Stacked circulation core**: one `Core` rect per building
   (`blueprint.core_size`, `domain::LAYOUT_CORE` draw, capped to about half
   the interior), shared by every storey so shafts run vertically; Top
   expands it by one tile (mechanical). Blueprints may opt into the legacy
   per-floor `LAYOUT_FLOOR` draw via `wandering_core`.
3. **Entrance (Ground only)**: a `Door` on the wall ring
   facing `ctx.door_side`, at a hashed offset along that edge. The cell just
   inside it is reserved as `Corridor` so the entrance
   always opens into the interior. Upper floors punch a lobby `Door` on the
   core edge instead — never a street door.
4. **Weighted room placement**: candidate anchors are
   every free cell, visited in a Fisher–Yates order from the `LAYOUT_ROOM`
   stream. At each anchor a template is rolled against `weight` via
   `roll_room` and a size within its `min`/`max`
   bounds is drawn; `try_place_room` then walks the
   size candidates closest to the roll and places the first that fits.
   `room_fits` requires the rectangle to be free and
   its one-tile margin to contain no other room — the margin, plus the fact
   rooms may hug walls and the core, is what keeps every room reachable.
   Rooms are painted with their opaque `kind` tag.
   When the rolled template cannot fit the remaining free area (typical on
   small lots), placement retries once with the blueprint's smallest template,
   so no lot degrades to a corridor-only shell.
5. **Lobby halo (Ground, after rooms)**: leftover `Void` around the shaft
   becomes `Corridor`, opening arrivals without ever stealing room cells.
6. **Doors**: the `LAYOUT_DOOR` stream rotates each
   room's perimeter (k=1 onward, after the entrance at k=0) and turns the
   first facing margin cell into a `Door`, so every room has exactly one
   opening into circulation.
7. **Corridor fill**: every leftover free cell becomes `Corridor`, so the
   margin channels connect all rooms, the core, and the street entrance into
   one navigable interior.

The result is deterministic, sealed (only entrance/room `Door`s break the
ring), vertically coherent (one shaft, one entrance, repeated typicals), and
every room opens onto circulation.

### Units, shafts, and mixed use (Milestone 14)

* **Subdivision** (`split_units`): the placeable region splits by guillotine
  cuts into ≤ `unit_max` apartment rects (single-loaded band excluded first).
  Cuts keep ≥3 cells per side and never orphan a unit from every plumbing
  shaft. Each unit gets its own anchor order, rooms, a forced room if left
  empty, and a front `Door` onto circulation.
* **Minimums then fill**: templates with `min_count` place first across
  units (round-robin, best-effort — unplaceable minimums are skipped, never
  retried forever); weighted rolls fill the rest; every room punches its
  template's `doors` count.
* **Wet stacks** (`wet_shaft_columns`, `domain::LAYOUT_WET`): building-wide
  shaft columns, identical every floor. WET rooms cover a shaft column or —
  in the fill phase — are skipped, so every placed wet room provably stacks;
  minimum passes allow off-shaft fallback so required kitchens still land.
* **Mixed-use ground** (`resolve_ground_override`,
  `generate_layout_with_ground`): 3+ storey buildings whose blueprint names
  `ground_zone` draw floor 0 from that zone's table (retail base under
  housing by default); the stacked shaft still comes from the main
  blueprint.
* **Corridor policy**: `corridor == 1` pre-paints a south band and confines
  rooms north of it (single-loaded); `0` is the classic double-loaded fill.

Covered by `units_partition_the_floor_and_hold_rooms`,
`minimums_hold_kitchen_and_bath_on_homes`,
`wet_rooms_stack_on_shaft_columns`, `mixed_use_ground_serves_retail`,
`multi_door_rooms_open_more_than_once`,
`single_loaded_policy_confines_rooms_north`,
`splits_keep_every_unit_on_a_shaft`, and
`unplaceable_minimums_skip_without_panic`.

## 8. Hash domains

Reserved for interior work (see `src/hash.rs:62` and `include/urbix.h`):

| Domain | Constant | Use |
|---|---|---|
| 40 | `LAYOUT_PICK` | blueprint selection (reserved) |
| 41 | `LAYOUT_FLOOR` | core placement; per-floor variation |
| 42 | `LAYOUT_ROOM` | room-kind rolls, anchor shuffle, size draws |
| 43 | `LAYOUT_ROOM_SIZE` | size draws (reserved; sizes draw on the room stream) |
| 44 | `LAYOUT_DOOR` | entrance pick (k=0) + room/core doors (k≥1) |
| 45 | `LAYOUT_FURNITURE` | slot density (reserved for M15) |
| 46 | `LAYOUT_CORE` | building-level shaft position (stacked cores) |

## 9. Interior FFI export (Milestone 10)

`src/ffi.rs` exposes a built cell's interior to any C-compatible consumer,
mirroring the chunk stream in shape and ownership. See the generated
`include/urbix.h` for the exact declarations.

### Requesting an interior

```c
UrbixInterior in = urbix_generate_interior(engine, wx, wz);
```

The engine locates the cell **from world coordinates alone** — no
chunk/cell-index juggling by the consumer. `(wx, wz)` is the canonical
`InteriorId` key: the engine derives the chunk with `div_euclid`/`rem_euclid`
on the configured chunk size, generates it if needed, and rebuilds the context
via `chunk::interior_context_for` before running `generate_layout` — the same
path the Rust APIs use, so C and Rust always agree on a lot's interior. A cell
with `height <= 0` (or a null engine handle) returns a zeroed record.

Always release a returned buffer:

```c
urbix_interior_free(in);   // null data is a safe no-op
```

### `UrbixInterior` payload layout

The record is a `#[repr(C)]` flat buffer (`src/ffi.rs`):

| Field | Type | Meaning |
|---|---|---|
| `interior_id` | `uint64_t` | stable interior key (the built cell's key) |
| `seed` | `uint64_t` | world seed used for generation |
| `zone` | `uint8_t` | dominant `ZoneType` index (0–4) |
| `door_side` | `uint8_t` | `DoorSide` index (0 west, 1 east, 2 north, 3 south) |
| `footprint_w` | `uint8_t` | shared floor-grid width in tiles |
| `footprint_d` | `uint8_t` | shared floor-grid depth in tiles |
| `floor_count` | `uint16_t` | number of storeys |
| `len` | `uint64_t` | payload byte length |
| `data` | `uint8_t *` | payload; null when unbuilt |

`data` holds one payload chunk per storey, in order `floor 0 … floor_count-1`:

```
tiles[footprint_w * footprint_d]   // Tile enum bytes, row-major
kinds[footprint_w * footprint_d]   // opaque room-kind tags (0 = not a room)
```

Tile bytes are the `Tile` enum values (`src/layout.rs:195`): 0 void, 1 wall,
2 door, 3 core, 4 corridor, 5 room. Because every floor shares the same grid,
`len = floor_count * 2 * footprint_w * footprint_d`. Room `kind`s are
consumer-meaningful tags the engine only passes through (blueprint
`BlueprintRoom.kind`), so a renderer can tint a "kitchen" differently from an
"office" without the engine knowing what either is.

### Streaming a city with interiors

Pair the interior query with the chunk stream from `docs/api.md` §8: for each
built cell you already rendered as an exterior box, `urbix_generate_interior`
fetches the matching room grid on demand when the player steps inside — the
engine caches the chunk, and interiors are re-derived cheaply per request
(no interior cache is crossed over the FFI yet).

## Roadmap — M13 Structure, M14 Program, M15 Finish (⬜ PENDING)

Locked decisions: structure before program; `InteriorContext` and the FFI
interior payload may grow (MINOR bumps, `Cell` stays 40 B); end state serves
furnished rooms + per-room metadata. Full milestone plan lives in
`Urbix_Project.md` §7 (M13–M15). Cross-cutting rules for all three:

* New hash domains per use (`FLOOR_ROLE`, `UNIT_SPLIT`, `WET_SHAFT`,
  `FURNISH`, …); floor folding stays in `floor_hash` (`src/interior.rs`).
* Same `(id, floor, seed)` → same bytes, asserted per feature.
* Serde defaults on new blueprint/context fields so old TOML/JSON parses.
* `docs/interiors.md` rewritten per milestone as each lands.

### M13 — Structure: make it a building

* `InteriorContext` gains lot truth recomputed pure from world coords in
  `chunk::interior_context_for` (real pack rect, corner flag, frontage
  depth, `building_role`, `secondary_zone`); `Cell` untouched.
* Floor roles Ground / Typical×N / Top; typical floors generate once and
  clone; stacked core drawn once per building; entrance door on Ground only
  (lobby doors off the core above).
* Tests: core identical across floors; typical byte-equal; one entrance on
  floor 0; context serde round-trip; FFI determinism still green.

### M14 — Program: rooms that make sense

* Blueprint v2 (same fixed-array pattern): per-room `min_count`, adjacency
  tags (WET/QUIET/PUBLIC/STREET_FACING), `doors`; corridor/unit/core
  policies; mixed-use ground override.
* Residential unit subdivision (guillotine splits → front doors → in-unit
  rooms with kitchen+bath minimums); wet-stack snapping to 1–2 vertical
  shafts; retail base under housing when commercial affinity is high.
* Tests: unit doors == units; wet tiles share ≤2 columns; minimums hold.

### M15 — Finish: lived-in + export

* Furniture on reserved `LAYOUT_FURNITURE` (per-kind templates, density
  knob); windows derived renderer-side first; additive
  `urbix_generate_interior_rooms` (rect/kind/area per room) leaving
  `UrbixInterior` untouched; FFI path routed through `InteriorCache`.
* Acceptance: extended `interior_report` + interiors gate example.

## Reference

- Data model: `src/layout.rs` (context, tiles, floors, blueprints, defaults).
- Generator + cache: `src/interior.rs` (`InteriorState`, `generate_layout`,
  `InteriorCache`).
- Config wiring: `src/config.rs` (`interior_floor_height`, `interior_max_floors`,
  `interior_blueprints`, `blueprint_for`, `interior_context`, `is_valid`).
- Cell → context: `src/chunk.rs:139` (`interior_context_for`).
- Default `interior_id`: `src/building.rs`, `domain::INTERIOR`.
- Headers: `docs/api.md`, `Urbix_Project.md §4.4 / §8.1`.