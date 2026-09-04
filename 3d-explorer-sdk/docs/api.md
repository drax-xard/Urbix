# Urbix Engine — C API Reference (v0.8.0, macos-aarch64)

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
- **flags** (is it a street? is it park?),
- an **interior key** (a deterministic id of the room inside the building,
  `0` = no interior).

The same `seed` always produces the *identical* city. Chunks are generated
**on demand** and LRU-cached, so memory stays bounded as you fly through an
infinite world.

> **Determinism invariant**: everything derives from
> `hash(x, y, seed, domain)`. There is no global RNG and no cross-chunk write
> dependency, so adjacent chunks agree at shared edges and the whole city is
> reproducible from a seed.

---

## 2. Getting the library

The SDK ships both forms in `sdk/`:

| File | Kind | Use when |
|---|---|---|
| `sdk/lib/liburbix.a` | **static** library | You want one self-contained binary (recommended for a demo) |
| `sdk/lib/liburbix.dylib` | dynamic library (macOS) | You want to hot-swap or link at runtime |
| `sdk/include/urbix.h` | C header | Everything here is declared in it |

The archive `sdk/urbix-0.8.0-macos-aarch64.tar.gz` is the exact CI release
artifact (matches `.github/workflows/release.yml`) with the same layout.

### Link line (macOS/AArch64)

```sh
cc -I sdk/include \
   -fsanitize=address 2>/dev/null || true   # optional
   my_program.c \
   sdk/lib/liburbix.a \
   -framework Security -framework CoreFoundation \
   -lm \
   -o my_program
```

The `-framework Security -framework CoreFoundation` and `-lm` flags are
required because the Rust runtime depends on them on macOS. If you link the
`.dylib` instead, add `-L sdk/lib -lurbix` and (for a self-contained load path)
set `DYLD_LIBRARY_PATH` at runtime or `install_name_tool` at build.

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
} ZoneParams;

typedef struct WorldConfig {
    uint64_t seed;
    uint16_t chunk_size;
    uint32_t draw_distance;
    uint16_t voronoi_site_count;
    double   voronoi_span;
    double   shepard_power;
    double   shepard_epsilon;
    double   zone_weights[ZONE_COUNT];       /* ZONE_COUNT == 5 */
    ZoneParams zones[ZONE_COUNT];
    uint8_t  zone_hues[ZONE_COUNT][3];       /* RGB per zone */
    uint16_t interior_width_range[2];
    uint16_t interior_height_range[2];
} WorldConfig;
```

To build a config from C, see §9. The struct is `#[repr(C)]` and its layout is
fixed for this version.

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
    InteriorId interior_id;         /* uint64; 0 = no interior */
} UrbixCell;
```

Cell flags (`CellFlags` is `uint8_t`):

```c
#define CellFlags_IS_STREET (1 << 0)   /* cell is a road; height 0, no building */
#define CellFlags_IS_PARK   (1 << 1)   /* park/green cell */
/* compat shims (same values): URBIX_FLAG_STREET, URBIX_FLAG_PARK */
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

---

## 4. Lifecycle functions

| Function | Purpose |
|---|---|
| `UrbixEngine *urbix_engine_create(uint64_t seed)` | Create an engine with default config + seed. Returns opaque handle, or NULL on failure. Caller owns it. |
| `void urbix_engine_destroy(UrbixEngine *engine)` | Release the engine. NULL is a no-op. Handle is dangling afterwards. |
| `UrbixEngine *urbix_engine_create_with_config(const WorldConfig *config)` | Create from a full config. Returns NULL if `config` is NULL or invalid. |

### Threading

A single `UrbixEngine` instance **must not** be used concurrently — it holds a
mutable chunk cache. Either guard it with a mutex, or give each thread its own
engine (schedule generation on a worker thread and hand results to the render
thread). See §8 for a recommended pattern.

---

## 5. Generation & query

### Generate a chunk

```c
UrbixChunkBuffer urbix_generate_chunk(UrbixEngine *engine, int32_t cx, int32_t cy);
```

- `cx`, `cy` are the **chunk** coordinates. For world cell coordinates `(wx, wz)`,
  the chunk is `cx = wx / chunk_size`, `cy = wz / chunk_size` (integer division,
  rounding toward zero).
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
        /* draw road at (wx, height 0, wz) */
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
Chunk `(0,0)`'s cell `(0,0)` is at world `(0,0)`; the world is 1-unit-per-cell,
so chunk_size 32 means each chunk spans 32 world units.

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

---

## 8. Streaming an infinite world

Because chunks are cached and evicted by Chebyshev distance, the canonical
"explorer" loop is:

1. Track the player's world position; derive the current chunk `(pcx, pcy)`.
2. For each chunk in the render radius `(r)` around `(pcx, pcy)`:
   - if the chunk isn't already loaded, `urbix_generate_chunk` and spawn its
     meshes/instances;
   - if a loaded chunk falls outside the radius, cull it (the engine evicts its
     cache entry automatically on the next generation).
3. Call `urbix_set_draw_distance(engine, r)` once so the cache capacity matches
   your visible radius; call it again if `r` changes (view distance slider).

Do generation on a **worker thread** with a mutex-guarded engine, or keep the
engine on the main thread and generate synchronously; don't block the render
frame on huge sweeps — spread chunk generation across frames.

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
/* zones: height_min, height_max, density, block_size, palette_count */
cfg.zones[0] = (ZoneParams){ 40.0f, 200.0f, 0.95f, 4, 6 }; /* Downtown */
cfg.zones[1] = (ZoneParams){  4.0f,  18.0f, 0.80f, 8, 5 }; /* Residential */
cfg.zones[2] = (ZoneParams){ 12.0f,  60.0f, 0.90f, 5, 7 }; /* Commercial */
cfg.zones[3] = (ZoneParams){  6.0f,  25.0f, 0.70f, 12, 4 };/* Industrial */
cfg.zones[4] = (ZoneParams){  0.0f,   2.0f, 0.10f, 16, 3 };/* Park */
/* zone_hues from DEFAULT_ZONE_HUES */
cfg.interior_width_range[0]  = 6;  cfg.interior_width_range[1]  = 14;
cfg.interior_height_range[0] = 6;  cfg.interior_height_range[1] = 14;

UrbixEngine *e = urbix_engine_create_with_config(&cfg);
```

`urbix_engine_create_with_config` returns NULL if the config is invalid
(bad ranges, zone weights not summing ~1.0, etc.), so validating via the C
API means checking for a non-NULL result.

> **Defaults**: if you only want a different seed, use
> `urbix_engine_create(seed)` — it uses the same defaults as the table above
> and is the simplest path.

---

## 10. Error-handling contract

Urbix's FFI **never panics into C**. On invalid inputs the functions no-op:

- `urbix_generate_chunk(NULL, …)` → `{NULL, 0}`.
- `urbix_chunk_free({NULL, 0})` → no-op.
- `urbix_engine_destroy(NULL)` → no-op.
- `urbix_set_*(NULL, …)` → no-op.
- Invalid `WorldConfig` → `create_with_config` returns NULL; `set_config` no-ops.

You should still treat a NULL engine handle from `create`/`create_with_config`
as an error (allocation or bad config).

**Ownership rules (must-haves):**
1. Caller owns `UrbixEngine`; destroy once, with `urbix_engine_destroy`.
2. Caller owns each successful `UrbixChunkBuffer.data`; free once, with
   `urbix_chunk_free` — **not** `free()`.
3. Never dereference `UrbixEngine`.

---

## 11. Quick reference table

| Call | In | Out |
|---|---|---|
| `urbix_engine_create(seed)` | `u64` | `UrbixEngine*` (or NULL) |
| `urbix_engine_create_with_config(&cfg)` | `const WorldConfig*` | `UrbixEngine*` (or NULL) |
| `urbix_engine_destroy(e)` | `UrbixEngine*` | – |
| `urbix_generate_chunk(e, cx, cy)` | `int32_t,int32_t` | `UrbixChunkBuffer` (free me) |
| `urbix_chunk_free(buf)` | `UrbixChunkBuffer` | – |
| `urbix_get_zone(e, wx, wz)` | `double,double` | `UrbixZoneAffinity` |
| `urbix_set_draw_distance(e, radius)` | `uint32_t` | – |
| `urbix_set_chunk_size(e, size)` | `uint16_t` | – (clears cache) |
| `urbix_set_config(e, &cfg)` | `const WorldConfig*` | – (regenerates) |
