# Urbix Engine — C API Reference (v0.17.0)

This is the authoritative reference for calling the Urbix procedural city
engine from C (or any language with C interop: C++, Rust, C#, Unity, Godot,
Python `ctypes`, etc.). Read this before writing an integration.

---

## 1. What the engine does

Urbix is a **deterministic, infinite procedural city generator**. Given a
`seed`, it produces an unbounded grid of **chunks**, each a square block of
**cells**. Every cell is a single city tile with:

- a building **height** (world units),
- a **zone affinity** vector (how strongly it belongs to each of 5 district
  types: Downtown, Residential, Commercial, Industrial, Park),
- a **palette index** (which facade color it uses),
- **flags** (street hierarchy: street / arterial / plaza / sidewalk /
  greenway / park),
- an **interior key** (a deterministic id of the room inside the building,
  `0` = no interior), and — since 0.10.0 — the full **interior layout** itself
  (per-storey tile grids + furniture, fetched with `urbix_generate_interior`)
  plus — since 0.15.0 — **per-room records** (rect, kind, apartment unit,
  area, via `urbix_generate_interior_rooms`).

The same `seed` always produces the *identical* city. Chunks are generated
**on demand** and LRU-cached, so memory stays bounded as you fly through an
infinite world. Scale canon: **1 cell = 4 m** (so a default 32-cell chunk
spans 128 m).

> **Determinism invariant**: everything derives from
> `hash(x, y, seed, domain)`. There is no global RNG and no cross-chunk write
> dependency, so adjacent chunks agree at shared edges and the whole city is
> reproducible from a seed.

---

## 2. Getting the library

The SDK ships both forms in `sdk/` (this build: `macos-aarch64`; the GitHub
Release carries `linux-x86_64` / `macos-aarch64` / `windows-x86_64` with the
same layout — swap `sdk/lib` + `sdk/include` to retarget):

| File | Kind | Use when |
|---|---|---|
| `sdk/lib/liburbix.a` (`urbix.lib` on Windows) | **static** library | You want one self-contained binary (recommended for a demo) |
| `sdk/lib/liburbix.dylib` (`.so` on Linux, `.dll` on Windows) | dynamic library | You want to hot-swap or link at runtime |
| `sdk/include/urbix.h` | C header | Everything here is declared in it |

The archive `sdk/urbix-0.15.0-<target>.tar.gz` (+ `.sha256`) is the exact CI
release artifact (matches `.github/workflows/release.yml`) with the same
layout.

### Link lines

```sh
# macOS
cc -I sdk/include my_program.c sdk/lib/liburbix.a \
   -framework Security -framework CoreFoundation -lm -o my_program
# Linux
cc -I sdk/include my_program.c sdk/lib/liburbix.a \
   -ldl -lm -pthread -o my_program
# Windows (MinGW)
cc -I sdk/include my_program.c sdk/lib/urbix.lib \
   -lws2_32 -luserenv -o my_program.exe
```

If you link the shared lib instead, add `-L sdk/lib -lurbix` and (for a
self-contained load path) set `DYLD_LIBRARY_PATH` / `LD_LIBRARY_PATH` at
runtime or `install_name_tool` at build.

---

## 3. Core types (from `urbix.h`)

### `UrbixEngine` — opaque handle

```c
typedef struct UrbixEngine UrbixEngine;
```

Opaque. You create it, pass it to every function, and destroy it. Never
dereference it.

### `WorldConfig` — all tunables

```c
typedef struct ZoneParams {
    float   height_min;
    float   height_max;
    float   density;
    uint8_t block_size;
    uint8_t palette_count;
    uint8_t arterial_every;   /* M11: every K-th street is a 2-cell avenue; < 2 disables */
} ZoneParams;

typedef struct BlueprintRoom {
    uint8_t kind;       /* opaque room-kind tag (consumer maps to living/kitchen/...) */
    float   weight;     /* relative selection weight */
    uint8_t min_w, max_w, min_d, max_d;  /* room size range in tiles */
    uint8_t min_count;  /* M14: minimum placements per floor */
    uint8_t tags;       /* M14: TAG_WET/QUIET/PUBLIC/STREET bits, 0 = none */
    uint8_t doors;      /* M14: doors per room (0 reads as 1) */
} BlueprintRoom;

typedef struct Blueprint {
    uint8_t margin;          /* wall-ring width in tiles */
    uint8_t core_size;       /* circulation-core width in tiles */
    uint8_t room_count;      /* live entries in rooms[] (0..8) */
    BlueprintRoom rooms[8];  /* MAX_BLUEPRINT_ROOMS == 8 */
    uint8_t vary_typical;    /* M13: nonzero = re-roll typical floors (default 0 = clone) */
    uint8_t wandering_core;  /* M13: nonzero = core wanders per floor (default 0 = stacked) */
    uint8_t unit_max;        /* M14: max apartments per floor (0 = open plan) */
    uint8_t wet_shafts;      /* M14: plumbing columns per building (0 = off, max 4) */
    uint8_t ground_zone;     /* M14: ground-floor blueprint override zone (255 = none) */
    uint8_t corridor;        /* M14: 0 = double-loaded, 1 = single-loaded band */
    uint8_t furn_density;    /* M15: second-furniture-piece probability 0..100 */
} Blueprint;

typedef struct WorldConfig {
    uint64_t seed;
    uint16_t chunk_size;            /* cells per side, e.g. 32 */
    uint32_t draw_distance;         /* chunk Chebyshev radius retained */
    uint16_t voronoi_site_count;    /* district sites, typically 24-48 */
    double   voronoi_span;          /* default 10000.0 */
    double   shepard_power;         /* default 4.0 */
    double   shepard_epsilon;       /* default 1e-8 */
    double   zone_weights[ZONE_COUNT];       /* ZONE_COUNT == 5, must sum ~1.0 */
    ZoneParams zones[ZONE_COUNT];
    uint8_t  zone_hues[ZONE_COUNT][3];       /* RGB per zone */
    uint16_t interior_width_range[2];        /* default 6..14 (legacy sizing) */
    uint16_t interior_height_range[2];       /* default 6..14 (legacy sizing) */
    float    interior_floor_height;          /* M9: world units per storey (default 4.0) */
    uint8_t  interior_max_floors;            /* M9: floor cap (default 64) */
    Blueprint interior_blueprints[ZONE_COUNT]; /* M9 rule tables, extended M13-M15 */
} WorldConfig;
```

To build a config from C, see §9. The struct is `#[repr(C)]` and its layout is
fixed for this version. New fields are always serde-defaulted, so old
TOML/JSON config files keep parsing.

Room tag bits (`BlueprintRoom.tags`):

```c
#define TAG_WET    (1 << 0)   /* snap to plumbing shafts */
#define TAG_QUIET  (1 << 1)   /* avoid street edges + core */
#define TAG_PUBLIC (1 << 2)   /* prefer the entrance half */
#define TAG_STREET (1 << 3)   /* prefer the outside wall ring */
```

Furniture codes (parallel `furn` layer, `0` = bare, set only on `Room` tiles):

```c
#define FURN_NONE 0
#define FURN_BED 1      /* bedrooms */
#define FURN_TABLE 2    /* living, meeting, work bays */
#define FURN_COUNTER 3  /* kitchens, retail, washrooms */
#define FURN_DESK 4     /* offices, lobbies, receptions */
#define FURN_SHELF 5    /* stockrooms, living rooms, utility */
#define FURN_BATH 6     /* bathrooms */
```

### `UrbixChunkBuffer` — generated chunk payload

```c
typedef struct UrbixChunkBuffer {
    uint8_t *data;   /* owned by caller; free with urbix_chunk_free */
    uint64_t len;    /* sizeof(header) + cell_count * sizeof(cell) */
} UrbixChunkBuffer;
```

Layout of `data`:

```
offset 0          : UrbixChunkHeader (32 bytes)
offset 32         : UrbixCell[cell_count]  (each 40 bytes, contiguous, no padding)
offset 32 + n*40  : total length == len
```

### `UrbixChunkHeader` — 32 bytes

```c
typedef struct UrbixChunkHeader {
    int32_t  cx;          /* chunk column (world / chunk_size) */
    int32_t  cy;          /* chunk row    (world / chunk_size) */
    uint32_t cell_count;  /* chunk_size * chunk_size */
    uint16_t chunk_size;  /* cells per side */
    uint8_t  _pad[6];
    uint64_t seed;        /* seed used to generate this chunk (verification) */
} UrbixChunkHeader;
```

### `UrbixCell` — 40 bytes

```c
typedef struct UrbixCell {
    float     height;               /* building height in world units; 0 = street/open */
    float     zone_affinity[5];     /* weights, sum ~ 1.0 */
    uint8_t   palette_id;           /* facade color index in owning zone */
    CellFlags flags;                /* bit flags below */
    uint16_t  _pad;
    InteriorId interior_id;         /* uint64; 0 = no interior (one key per lot) */
} UrbixCell;
```

Cell flags (`CellFlags` is `uint8_t`; additive — `Cell` stays 40 B):

```c
#define CellFlags_IS_STREET   (1 << 0)  /* road; height 0, no building */
#define CellFlags_IS_PARK     (1 << 1)  /* park/green cell */
#define CellFlags_IS_ARTERIAL (1 << 2)  /* M11: 2-cell avenue (always + IS_STREET) */
#define CellFlags_IS_PLAZA    (1 << 3)  /* M11: pedestrian plaza (+ IS_STREET) */
#define CellFlags_IS_SIDEWALK (1 << 4)  /* M11: 1-cell sidewalk ring, no-build */
#define CellFlags_IS_GREENWAY (1 << 5)  /* M12: dropped street reborn as linear park */
/* compat shims (same values): URBIX_FLAG_STREET/PARK/ARTERIAL/PLAZA/SIDEWALK/GREENWAY */
```

### `UrbixZoneAffinity` — continuous zone query result

```c
typedef struct UrbixZoneAffinity {
    float weights[5];
} UrbixZoneAffinity;
```

The 5 zones, in index order (this is the `ZONE_COUNT == 5` convention):

| Index | Zone |
|---|---|
| 0 | Downtown (steel-blue hue, tall) |
| 1 | Residential (tree green) |
| 2 | Commercial (warm orange) |
| 3 | Industrial (grimy grey/brown) |
| 4 | Park (light green) |

### `UrbixInterior` — generated building interior (since 0.10.0, furniture since 0.15.0)

```c
typedef struct UrbixInterior {
    uint64_t  interior_id;     /* interior key (the built cell's key) */
    uint64_t  seed;            /* world seed used for generation */
    uint8_t   zone;            /* dominant zone index 0-4 */
    uint8_t   door_side;       /* 0 west, 1 east, 2 north, 3 south */
    uint8_t   footprint_w;     /* floor-grid width in tiles */
    uint8_t   footprint_d;     /* floor-grid depth in tiles */
    uint16_t  floor_count;     /* number of storeys */
    uint64_t  len;             /* payload byte length; 0 when unbuilt */
    uint8_t  *data;            /* owned by caller; free with urbix_interior_free */
} UrbixInterior;
```

`data` holds one payload chunk per storey, in floor order. Each storey:

```
tiles[footprint_w * footprint_d]   /* Tile enum bytes, row-major */
kinds[footprint_w * footprint_d]   /* opaque room-kind tags (0 = not a room) */
furn[footprint_w * footprint_d]    /* furniture codes FURN_* (0 = bare; only on Room tiles) */
```

Tile bytes (`Tile` enum): `0` void, `1` wall, `2` door, `3` core (stairs/elevator),
`4` corridor, `5` room. Every floor shares the same footprint, so
`len == floor_count * 3 * footprint_w * footprint_d`. The furniture layer is
**appended, never interleaved**: readers slicing the first two thirds keep
working unchanged. An unbuilt cell (`height <= 0`) or NULL engine yields a
zeroed record with `data == NULL`.

Vertical structure (M13+): one stacked circulation shaft per building, floor
roles Ground / Typical×N (typical generates once and clones) / Top, and a
ground-only street entrance with lobby. Room programs (M14+): apartments with
front doors, wet rooms snapped to 1–2 plumbing shafts, mixed-use retail bases
under housing.

### `UrbixRoom` / `UrbixRoomList` — per-room records (since 0.15.0)

```c
typedef struct UrbixRoom {
    uint8_t  floor;   /* storey index */
    uint8_t  x, z;    /* left/top edge in grid cells */
    uint8_t  w, d;    /* width/depth in cells */
    uint8_t  kind;    /* opaque blueprint room tag */
    uint8_t  unit;    /* apartment index on this floor (255 = outside every unit) */
    uint16_t area;    /* floor area in tiles */
} UrbixRoom;          /* 10 bytes, header-asserted */

typedef struct UrbixRoomList {
    UrbixRoom *data;  /* owned by caller; free with urbix_interior_rooms_free */
    uint64_t   count; /* number of records, row-major scan order */
} UrbixRoomList;
```

Generated from the same finished floors `urbix_generate_interior` ships, so
grids and records always agree. Unbuilt cells and NULL engines yield
`{NULL, 0}`.

---

## 4. Lifecycle functions

| Function | Purpose |
|---|---|
| `UrbixEngine *urbix_engine_create(uint64_t seed)` | Create an engine with default config + seed. Returns opaque handle, or NULL on failure. Caller owns it. |
| `void urbix_engine_destroy(UrbixEngine *engine)` | Release the engine. NULL is a no-op. Handle is dangling afterwards. |
| `UrbixEngine *urbix_engine_create_with_config(const WorldConfig *config)` | Create from a full config. Returns NULL if `config` is NULL or invalid. |

### Threading

A single `UrbixEngine` instance **must not** be used concurrently — it holds a
mutable chunk cache (and, since 0.15.0, an interior cache). Either guard it
with a mutex, or give each thread its own engine (schedule generation on a
worker thread and hand results to the render thread). See §8 for a recommended
pattern.

---

## 5. Generation & query

### Generate a chunk

```c
UrbixChunkBuffer urbix_generate_chunk(UrbixEngine *engine, int32_t cx, int32_t cy);
```

- `cx`, `cy` are the **chunk** coordinates. For world cell coordinates `(wx, wz)`,
  the chunk is `cx = div_euclid(wx, chunk_size)`, `cy = div_euclid(wz, chunk_size)`
  (Euclidean division — negative worlds map correctly).
- On success returns a buffer you MUST release with `urbix_chunk_free`.
- On failure returns `{data: NULL, len: 0}`.

### Free a chunk buffer

```c
void urbix_chunk_free(UrbixChunkBuffer buf);
```

- NULL data is a no-op.
- **Never** free with the C `free()` — the buffer is Rust-allocated. You must
  call `urbix_chunk_free`, exactly once.

### Query zone affinity at a world point

```c
UrbixZoneAffinity urbix_get_zone(UrbixEngine *engine, double wx, double wz);
```

Return the **continuous** blended zone weight vector (sum ~1.0) at continuous
world coordinates `(wx, wz)`. Useful for ground colour, ambient audio, or
spawning district-specific props. The `wx/wz` are world units in the same frame
as cell positions (i.e. a cell's world position is `x = cx*chunk_size + l`, etc.).

### Generate an interior (since 0.10.0)

```c
UrbixInterior urbix_generate_interior(UrbixEngine *engine, int32_t wx, int32_t wz);
```

- `wx`, `wz` are **world cell coordinates** (the canonical interior key). The
  engine derives the chunk with `div_euclid`/`rem_euclid` on the configured
  chunk size, generates it if needed, and rebuilds the cell's context via the
  same path the Rust APIs use — so a C consumer and the Rust examples always
  agree on a lot's interior.
- Requires a built cell (`height > 0`). Otherwise returns `{ ...len: 0, data: NULL }`.
- On success returns a record you MUST release with `urbix_interior_free`.
- NULL engine → zeroed record.
- Both interior entry points route through the engine-side `InteriorCache`
  (0.15.0), so repeat visits are cheap.

### Free an interior buffer

```c
void urbix_interior_free(UrbixInterior interior);
```

- NULL `data` is a no-op.
- **Never** free with the C `free()` — the buffer is Rust-allocated. You must
  call `urbix_interior_free`, exactly once.

### Enumerate rooms (since 0.15.0)

```c
UrbixRoomList urbix_generate_interior_rooms(UrbixEngine *engine, int32_t wx, int32_t wz);
void urbix_interior_rooms_free(UrbixRoomList list);
```

- Same lookup path as `urbix_generate_interior` (chunk → context → layout,
  through the interior cache), then one `UrbixRoom` per 4-connected room
  component per floor, with apartment and area attached.
- NULL `data` free is a no-op; **never** use C `free()`; free exactly once.
- NULL engine or unbuilt cell → `{NULL, 0}`.

---

## 6. Runtime setters

| Function | Effect |
|---|---|
| `void urbix_set_draw_distance(UrbixEngine *engine, uint32_t radius)` | Chunk Chebyshev radius retained in cache before eviction. |
| `void urbix_set_chunk_size(UrbixEngine *engine, uint16_t size)` | Cells per chunk side for *subsequent* generation; **clears the cache** (old-size chunks are invalid). `0` is a no-op (never panics across FFI). |
| `void urbix_set_config(UrbixEngine *engine, const WorldConfig *config)` | Replace config wholesale; regenerates Voronoi + clears cache. No-op if NULL/invalid. |

---

## 7. Reading chunk data into your scene

The canonical loop to turn a chunk into meshes/instances:

```c
UrbixChunkBuffer buf = urbix_generate_chunk(engine, cx, cy);

const UrbixChunkHeader *hdr = (const UrbixChunkHeader *)buf.data;
const UrbixCell *cells = (const UrbixCell *)(buf.data + sizeof(UrbixChunkHeader));

for (uint32_t i = 0; i < hdr->cell_count; ++i) {
    const UrbixCell *c = &cells[i];

    uint32_t lx = i % hdr->chunk_size;      /* local x in chunk */
    uint32_t ly = i / hdr->chunk_size;      /* local y in chunk  */
    double   wx = (double)hdr->cx * hdr->chunk_size + lx;  /* world x */
    double   wz = (double)hdr->cy * hdr->chunk_size + ly;  /* world z */

    if (c->flags & CellFlags_IS_STREET) {
        /* Paved ground at (wx, height 0, wz). Branch further:
           IS_ARTERIAL = wide avenue, IS_PLAZA = warm plaza,
           IS_SIDEWALK = pale curb; plain IS_STREET = local road. */
        continue;
    }
    if (c->flags & CellFlags_IS_GREENWAY) {
        /* Linear park (draw green, like IS_PARK). */
        continue;
    }
    if (c->height > 0.0f) {
        /* draw a box from (wx, 0, wz) with height c->height;
           colour from zone_hues[dominant_zone] shaded by palette_id */
    }
    /* c->interior_id != 0 => deterministic room key for a doorway */
}

urbix_chunk_free(buf);
```

**Cell world position**: cell `i` in chunk `(cx, cy)` sits at
`wx = cx*chunk_size + i%chunk_size`, `wz = cy*chunk_size + i/chunk_size`.
Chunk `(0,0)`'s cell `(0,0)` is at world `(0,0)`; the world is 1-unit-per-cell
(scale canon: 1 cell = 4 m, so multiply by 4 for metres), chunk_size 32 means
each chunk spans 32 world units (128 m).

**Extra cells**: `cell_count` may exceed `chunk_size*chunk_size` if the engine
ever pads (it currently doesn't) — always iterate `cell_count`, never assume it
equals exactly the grid. Compute `lx/ly` with modulo/division as above so the
mapping stays correct regardless.

**Dominant zone** of a cell (for hue selection):

```c
int dominant_zone = 0;
for (int z = 1; z < ZONE_COUNT; ++z)
    if (c->zone_affinity[z] > c->zone_affinity[dominant_zone])
        dominant_zone = z;
```

See `examples/explore_grid.c` for the full worked loop including the
street-hierarchy branches.

---

## 8. Streaming an infinite world

Because chunks are cached and evicted by Chebyshev distance, the canonical
"explorer" loop is:

1. Track the player's world position; derive the current chunk `(pcx, pcy)`
   with Euclidean division by `chunk_size`.
2. For each chunk in the render radius `(r)` around `(pcx, pcy)`:
   - if the chunk isn't already loaded, `urbix_generate_chunk` and spawn its
     meshes/instances;
   - if a loaded chunk falls outside the radius, cull it (the engine evicts its
     cache entry automatically on the next generation).
3. Call `urbix_set_draw_distance(engine, r)` once so the cache capacity matches
   your visible radius; call it again if `r` changes (view distance slider).

Do generation on a **worker thread** with a mutex-guarded engine, or keep the
engine on the main thread and generate synchronously; don't block the render
frame on huge sweeps — spread chunk generation across frames (or fetch one
batch `/api/chunks?r=` per frame from the demo server).

**Interiors while flying:** for each built cell you already boxed
(`c->interior_id != 0`), call `urbix_generate_interior(engine, wx, wz)` on demand
when the player steps inside (or a picking ray hits the lot). The engine caches
the chunk *and* the interior, so the request is cheap; the returned storey grids
(tiles + kinds + furniture) let you render walls, the corridor/net core, rooms,
and furniture per floor (see `UrbixInterior`), while
`urbix_generate_interior_rooms` gives you labelled room rects for UI.

---

## 9. Building a `WorldConfig` from C

`WorldConfig` is plain data, so you can build it field-by-field. The header
also declares `DEFAULT_ZONE_HUES`. E.g.:

```c
WorldConfig cfg;
memset(&cfg, 0, sizeof(cfg));
cfg.seed                  = 445566;
cfg.chunk_size            = 32;
cfg.draw_distance         = 8;
cfg.voronoi_site_count    = 30;
cfg.voronoi_span          = 10000.0;
cfg.shepard_power         = 4.0;
cfg.shepard_epsilon       = 1e-8;
/* zone_weights: {0.25,0.30,0.20,0.15,0.10} */
cfg.zone_weights[0] = 0.25f; cfg.zone_weights[1] = 0.30f;
cfg.zone_weights[2] = 0.20f; cfg.zone_weights[3] = 0.15f;
cfg.zone_weights[4] = 0.10f;
/* zones: height_min, height_max, density, block_size, palette_count, arterial_every */
cfg.zones[0] = (ZoneParams){ 24.0f, 110.0f, 0.95f, 11, 6, 4 }; /* Downtown */
cfg.zones[1] = (ZoneParams){  4.0f,  14.0f, 0.80f, 10, 5, 5 }; /* Residential */
cfg.zones[2] = (ZoneParams){ 10.0f,  45.0f, 0.90f,  9, 7, 4 }; /* Commercial */
cfg.zones[3] = (ZoneParams){  6.0f,  20.0f, 0.70f, 14, 4, 6 }; /* Industrial */
cfg.zones[4] = (ZoneParams){  0.0f,   2.0f, 0.10f, 18, 3, 0 }; /* Park (no arterials) */
/* zone_hues from DEFAULT_ZONE_HUES */
cfg.interior_width_range[0]  = 6;  cfg.interior_width_range[1]  = 14;
cfg.interior_height_range[0] = 6;  cfg.interior_height_range[1] = 14;
cfg.interior_floor_height = 4.0f;
cfg.interior_max_floors  = 64;
/* interior_blueprints: leave zeroed for engine defaults, or fill per zone */

UrbixEngine *e = urbix_engine_create_with_config(&cfg);
```

`urbix_engine_create_with_config` returns NULL if the config is invalid
(bad ranges, zone weights not summing ~1.0, `arterial_every > 16`,
`furn_density > 100`, etc.), so validating via the C API means checking for
a non-NULL result.

> **Defaults**: if you only want a different seed, use
> `urbix_engine_create(seed)` — it uses the same defaults as the table above
> and is the simplest path. To tune a few fields, start from
> `urbix_default_config()`, patch what you need, and pass it to
> `urbix_engine_create_with_config` (invalid configs return NULL).

---

## 10. Error-handling contract

Urbix's FFI **never panics into C**. On invalid inputs the functions no-op:

- `urbix_generate_chunk(NULL, …)` → `{NULL, 0}`.
- `urbix_generate_interior(NULL, …)` → zeroed record (`data == NULL`, `len == 0`).
- `urbix_generate_interior_rooms(NULL, …)` → `{NULL, 0}`.
- `urbix_chunk_free({NULL, 0})` → no-op.
- `urbix_interior_free({…, NULL, 0})` → no-op.
- `urbix_interior_rooms_free({NULL, 0})` → no-op.
- `urbix_engine_destroy(NULL)` → no-op.
- `urbix_set_*(NULL, …)` → no-op.
- Invalid `WorldConfig` → `create_with_config` returns NULL; `set_config` no-ops.

You should still treat a NULL engine handle from `create`/`create_with_config`
as an error (allocation or bad config).

**Ownership rules (must-haves):**
1. Caller owns `UrbixEngine`; destroy once, with `urbix_engine_destroy`.
2. Caller owns each successful `UrbixChunkBuffer.data`; free once, with
   `urbix_chunk_free` — **not** `free()`.
3. Caller owns each successful `UrbixInterior.data`; free once, with
   `urbix_interior_free` — **not** `free()`.
4. Caller owns each successful `UrbixRoomList.data`; free once, with
   `urbix_interior_rooms_free` — **not** `free()`.
5. Never dereference `UrbixEngine`.

---

## 11. Quick reference table

| Call | In | Out |
|---|---|---|
| `urbix_engine_create(seed)` | `u64` | `UrbixEngine*` (or NULL) |
| `urbix_engine_create_with_config(&cfg)` | `const WorldConfig*` | `UrbixEngine*` (or NULL) |
| `urbix_default_config()` | – | `WorldConfig` (patch + pass to create/set) |
| `urbix_engine_destroy(e)` | `UrbixEngine*` | – |
| `urbix_generate_chunk(e, cx, cy)` | `int32_t,int32_t` | `UrbixChunkBuffer` (free me) |
| `urbix_chunk_free(buf)` | `UrbixChunkBuffer` | – |
| `urbix_generate_interior(e, wx, wz)` | `int32_t,int32_t` | `UrbixInterior` (free me) |
| `urbix_interior_free(in)` | `UrbixInterior` | – |
| `urbix_generate_interior_rooms(e, wx, wz)` | `int32_t,int32_t` | `UrbixRoomList` (free me) |
| `urbix_interior_rooms_free(list)` | `UrbixRoomList` | – |
| `urbix_get_zone(e, wx, wz)` | `double,double` | `UrbixZoneAffinity` |
| `urbix_set_draw_distance(e, radius)` | `uint32_t` | – |
| `urbix_set_chunk_size(e, size)` | `uint16_t` | – (clears cache) |
| `urbix_set_config(e, &cfg)` | `const WorldConfig*` | – (regenerates) |
