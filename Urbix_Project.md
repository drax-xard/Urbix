# Project Design — Urbix

A platform-agnostic and language-agnostic modular engine for generating an explorable, infinite procedural city that can be used/called from other engines either via CLI or API.

---

## 1. Objectives

### Design objectives
1. **A convincing, varied skyline.** Distinct urban regions — skyscraper
   business districts, tranquil residential areas, busy commercial zones,
   dirty industrial zones, and green parks — each with its own density,
   building height distribution, and color scheme.
2. **Infinite world, deterministic generation.** The city is generated in
   chunks on demand, so it is effectively unbounded while
   remaining reproducible from a seed.


### Performance objectives
3. **High-performance realtime.** 
4. **Bounded memory.** Chunks are cached with a distance-based eviction limit
   so infinite generation never grows unbounded in RAM.

### Engineering objectives
5. **Extensible and modular.** Core generation engine is transparently extensible with modules to alter or expand its capabilities.
6. **Clean and commented code :** Uses best coding practices to generate sensible code with high quality comments, verbose logging and useful test sets 
7. **Truly agnostic :** The engine only objective is to generate the city; other aspects like rendering, UI, gameplay and everything else is abstracted to whatever software is making use of this engine.

---

## 2. Architecture Overview

The engine is structured as a single Rust crate with a clean internal module
boundary and a C FFI surface exposed from the start.

### 2.1 Module map

| File | Role |
|---|---|
| `src/lib.rs` | Crate root; re-exports public API types. |
| `src/ffi.rs` | `#[no_mangle] extern "C"` entry points. Thin wrappers that delegate to `engine`. |
| `src/engine.rs` | `WorldEngine` — stateful handle holding config, cache, and Voronoi sites. |
| `src/data.rs` | All core data types (`Cell`, `Chunk`, `ChunkHeader`, `ChunkId`, `InteriorId`, `CellFlags`). All public types are `#[repr(C)]`. |
| `src/config.rs` | Tunables: `ChunkSize`, `DrawDistance`, `VoronoiSiteCount`, `Seed`. |
| `src/zones.rs` | `ZoneType` enum (Downtown, Residential, Commercial, Industrial, Park), per-zone parameter structs, color palettes. |
| `src/region.rs` | Voronoi diagram generation from seed, nearest-site query, fuzzy border blending → zone affinity vector. |
| `src/chunk.rs` | Orchestrates chunk generation: queries zone affinity at each cell, delegates to `street` and `building`. |
| `src/cache.rs` | LRU cache keyed by `ChunkId` with distance-based eviction. |
| `src/street.rs` | Street grid layout and block subdivision, tuned per zone. |
| `src/building.rs` | Building footprint detection, height assignment, palette selection. |
| `src/interior.rs` | `InteriorId` computation, `InteriorState` trait, stub `generate_interior`. |
| `src/hash.rs` | Deterministic seeded hashing: `hash(x, y, seed, domain) → u64`. |
| `include/` | Auto-generated C header (`urbix.h`) via `cbindgen` at build time. |

### 2.2 Data flow

```
seed
 │
 ▼
region.rs  ──▶  Voronoi sites (immutable, lives for entire run)
 │
 ├─ query(zone_affinity, world_x, world_z) ──▶ nearest 2 sites → blended zone params
 │
 ▼
chunk.rs  ──▶  for each cell (cx·CS .. cx·CS+CS, cy·CS .. cy·CS+CS):
 │                 zone_affinity → street.rs → is_street?  →  height = 0
 │                              → building.rs → height, palette_id, interior_id
 │
 ▼
Cell array (repr(C))  ──▶  binary Chunk wire format
 │
 ▼
cache.rs  ──▶  LRU insert, evict distant chunks
 │
 ▼
ffi.rs  ──▶  extern "C" functions → consumer (renderer / game engine)
```

### 2.3 Binary wire format

Every chunk is transmitted as a flat `repr(C)` byte buffer. The header is
followed by `cell_count` cell records, with no padding between records.

```c
// include/urbix.h (generated, manual reference)
typedef struct {
    int32_t   cx;            // chunk column index
    int32_t   cy;            // chunk row index
    uint32_t  cell_count;    // total cells (chunk_size × chunk_size)
    uint16_t  chunk_size;    // e.g. 32
    uint8_t   _pad[6];      // alignment padding
    uint64_t  seed;          // world seed for verification
} UrbixChunkHeader;

typedef struct {
    float     height;                    // building height (0 = street/open)
    float     zone_affinity[5];          // weight per zone type
    uint8_t   palette_id;               // facade color index
    uint8_t   flags;                    // bit 0 = is_street, bit 1 = is_park
    uint16_t  _pad;                     // alignment
    uint64_t  interior_id;              // deterministic interior key (0 = none)
} UrbixCell;
```

The total buffer size is `sizeof(UrbixChunkHeader) + cell_count * sizeof(UrbixCell)`.

### 2.4 C FFI surface

The FFI layer is intentionally thin. Each function maps 1:1 to an engine
method. All pointers passed in/out are C-compatible; no Rust allocator leaks
to the consumer.

```c
// Core lifecycle
UrbixEngine* urbix_engine_create(uint64_t seed);
void         urbix_engine_destroy(UrbixEngine* engine);

// Chunk generation — caller must free with urbix_chunk_free()
UrbixChunkBuffer urbix_generate_chunk(UrbixEngine* e, int32_t cx, int32_t cy);
void             urbix_chunk_free(UrbixChunkBuffer buf);

// Zone query — world coordinates are double (f64) to match the engine and
// stay precise over the engine's wide (i64) coordinate span.
UrbixZoneAffinity urbix_get_zone(UrbixEngine* e, double wx, double wz);

// Configuration
void urbix_set_draw_distance(UrbixEngine* e, uint32_t radius);
void urbix_set_chunk_size(UrbixEngine* e, uint16_t size);
```

---

## 3. World Generation — Voronoi Regions with Fuzzy Borders

Generation is layered so that *regions* (big, smooth) and *chunks* (local,
deterministic) compose into one seamless infinite city.

### 4.1 Region layer (districts)
- A **Voronoi diagram** of a fixed set of sites (e.g., 24–48) spread across a
  large coordinate span, computed once from the seed. Each site is tagged with
  a zone type from the following set:
  - **Downtown / Business** — dense, tall skyscrapers.
  - **Residential** — tranquil, low-rise, tree-lined.
  - **Commercial** — busy mid-rise with bright/neon palettes.
  - **Industrial** — grimy, wide low warehouses, open lots.
  - **Park / Green** — low or zero buildings, foliage, open ground.
- **Fuzzy (soft) borders:** each query blends contributions from the sites
  using a continuous (Shepard) inverse-distance weighting. Every site's weight
  is `1 / d^p` for a small power `p`, then the per-zone weights are
  normalised. The nearest site dominates deep inside its cell and the affinity
  falls off continuously toward the border. Because every weight is a
  continuous function of position, the result has **no hard edges and no
  identity snapping** even where three or more cells meet. The output is a
  per-point *zone affinity vector* across a small palette of parameters
  (density, height range, color palette, street/block style). This yields
  **gradual, seamless transitions** between adjacent districts and naturally
  keeps neighboring chunks consistent with one another.

### 4.2 Chunk layer
- World is divided into fixed **chunks** (e.g., 32×32 cells), addressed by
  integer `(cx, cy)` chunk coordinates.
- Each chunk's contents are generated **deterministically** from
  `hash(cx, cy, seed, zone_param_blob)`. No persistent global RNG and no
  cross-chunk write dependency — any chunk can be (re)built on demand and
  adjacent chunks agree at their shared edges because they query the same
  continuous Voronoi/zone field.
- **Street layout & blocks:** a per-zone street grid with block sizes tuned by
  region (tight small blocks downtown, roomy blocks residential, large wide
  blocks industrial). Streets have `height = 0`; interior of each block is
  filled with a building footprint (or open/park cells).
- **Per-building variation:** individual building height and facade
  palette-id are derived by hashing cell coordinates, clamped into the zone's
  height range, so every built cell looks a little different.

### 4.3 Cache & eviction
- Chunks are materialized lazily around the player and kept in an LRU cache
  keyed by `ChunkId`.
- Chunks far outside the draw distance are **evicted** (and dropped) so memory
  stays bounded even as the player explores infinitely.
- The *region* Voronoi map is tiny and immutable; it lives for the whole run.

### 4.4 Interior hooks (future)
Every built lot (each building/home/factory footprint) is a candidate for a
future interactable interior. To make that evolution cheap and seamless, the
world model **already computes an `InteriorId` for every built cell today**,
even though interiors are not yet rendered:

- `InteriorId = hash(cell_coords, seed, zone)` is deterministic and stable, so
  a given doorway always leads to the *same* room across visits and across
  runs with the same seed.
- The `interior` module defines the **hook surface** (function signatures /
  trait) a future renderer and teleport routine will implement:
  - `InteriorState` — what an interior run needs (room layout, size, fog,
    palette, exits).
  - `enter(lot, player)` / `exit(lot, player)` — teleport into/out of a
    deterministic interior.
  - `generate_interior(id) -> InteriorState` — stub returning a placeholder
    so the *interface* is wired and callable end-to-end.
- Until implemented, `interior` exposes a **null/generic `InteriorState`** and
  the enter action is a no-op (or clearly logged), so the data plumbing and
  the player-interface affordance exist without the feature being shown.
- Design note: an interior is a *separate mini-world* (its own small grid,
  not part of the infinite chunk city), keyed by `InteriorId`, so it can be
  generated and cached independently of the outdoor chunks.

---

## 5. Versioning & changelog
The project tracks a **semantic versioning** (SemVer) scheme and maintains a
`CHANGELOG.md`:

- **Version format** `MAJOR.MINOR.PATCH`:
  - `MAJOR` — breaking changes to the build.
  - `MINOR` — backward-compatible features (new zone, new mode, new flag).
  - `PATCH` — bug fixes and polish with no new surface.
- `CHANGELOG.md` follows the **Keep a Changelog** convention:
  - `[Unreleased]` section at top collects pending changes.
  - Categorized entries: `Added`, `Changed`, `Fixed`, `Removed`,
    `Performance`, `Documentation`.
  - Each milestone and each significant feature gets a tagged release.

---

## 6. Documentation & Code Comments

The codebase carries **quality, informative documentation** at every layer so
it is approachable and self-explanatory.

### 6.1 Doc comments (rustdoc)
- Every **public item** (module, routine, function, and
  significant field) gets a comment explaining *what* it does, *why*
  it exists, and (where non-obvious) *how* to use it.
- Modules open with a comment block describing their role in the overall
  architecture (mirroring the module map in section 2).
- Doc comments include an *example* where it clarifies usage.
- Intended audiences: a new contributor should be able to read the documentation and
  understand the whole pipeline without reading the source.

### 6.2 Inline comments
- Use **explanatory comments for the *why* and the *algorithm*** — especially
  in the deterministic generators, where the math is otherwise opaque.
- **Explain every nontrivial step** of the hardest parts to re-derive.
- Prefer naming + structure over comment walls: a helper with a clear name and
  a one-line doc beats a five-line bullet comment.
- **Do not** restate the obvious (`// increment i`); comments justify
  non-obvious decisions, edge cases, and invariants.
- Mark intentional placeholder code clearly with `TODO`/`FIXME` so the hooks and placeholder sections are
  visible to future work without being mistaken for finished features.

### 6.3 Project-level documentation
- `project-design.md` — this document: objectives, architecture, and the
  development plan (living document, updated as decisions change).
- `README.md` — the top-level entry point: what the project is, a screenshot
  description, quickstart (build + run), CLI flags, and a pointer to
  the design doc and changelog.
- `CHANGELOG.md` — versioned history.
- `docs/` — deeper write-ups (world generation, api structure, etc)
  created with the milestones, referenced from the README.

---

## 7. Development Plan (milestones)

Each milestone produces a compilable, testable increment. Work is ordered so
that earlier milestones are dependencies for later ones. Each milestone header
is prefixed with its status: ✅ **DONE** (implemented and released) or ⬜
**PENDING** (not yet started).

---

### Milestone 1 — Data Layer & Hashing — ✅ DONE (released in 0.2.0)

**Goal:** define every core data type and the deterministic hash primitive
that the rest of the engine is built on.

| File | Deliverable |
|---|---|
| `src/config.rs` | `WorldConfig` struct: seed, chunk_size (default 32), draw_distance, voronoi_site_count. All fields `#[repr(C)]`. |
| `src/hash.rs` | `fn hash_coords(x: i32, y: i32, seed: u64, domain: u8) -> u64` — wyhash or SipHash with domain separation byte. |
| `src/zones.rs` | `ZoneType` enum (5 variants), `ZoneParams` struct (height_min, height_max, density, block_size, palette). `fn zone_params(affinity: &[f32;5]) -> ZoneParams` that blends. |
| `src/data.rs` | `Cell`, `ChunkHeader`, `ChunkBuffer` (owned vec wrapper), `ChunkId`, `InteriorId`, `CellFlags` bitflags. All `#[repr(C)]`. |

**Tests:**
- `hash_coords` is deterministic: same input → same output across runs.
- `hash_coords` differs across all four domains for the same (x, y, seed).
- `zone_params` blend is correct at exact affinity boundaries.

**Exit criteria:** `cargo test` green, `cargo clippy` clean.

---

### Milestone 2 — Voronoi Region Layer — ✅ DONE

**Goal:** generate the Voronoi diagram from the seed and expose a fuzzy
zone-affinity query at any world coordinate.

| File | Deliverable |
|---|---|
| `src/region.rs` | `VoronoiDiagram` struct. `fn generate(seed, site_count) -> Self`. `fn query(world_x, world_z) -> [f32; 5]` returns zone affinity. |

**Algorithm:**
1. Hash `seed` to produce `site_count` (24–48) random 2D points spread over
   a large coordinate span (e.g. ±10 000 units).
2. Each site is tagged with a `ZoneType` chosen via weighted random from the
   same seed stream.
3. `query(x, z)` computes a **Shepard inverse-distance blend** over all sites:
   each site's weight is `1 / d^p` (with a tiny epsilon guard against landing
   exactly on a site), accumulated per zone and normalised. Because every
   weight is a continuous function of position, the affinity is continuous
   everywhere — the nearest site dominates deep inside its cell and there is
   no discontinuity even at points where several cells meet.

**Tests:**
- Query is deterministic across multiple calls.
- Query at a site's exact position returns a near-1.0 affinity for that zone.
- Two queries 0.01 apart yield affinities within a small ε (continuity),
  including sweeps across site-pair bisectors (the worst case for
  discontinuities).

**Exit criteria:** unit tests pass, fuzz-continuous property holds.

---

### Milestone 3 — Chunk Generation (Core Loop) — ✅ DONE

**Goal:** a chunk of `chunk_size × chunk_size` cells can be generated
deterministically from `(cx, cy, seed)`.

| File | Deliverable |
|---|---|
| `src/chunk.rs` | `fn generate_chunk(cx, cy, config, voronoi) -> ChunkBuffer`. |
| `src/street.rs` | `fn layout_block(cell_x, cell_y, zone_params) -> CellFlags`. Returns `is_street` based on per-zone block grid. |
| `src/building.rs` | `fn assign_building(cell_x, cell_y, zone_params, seed) -> (f32 height, u8 palette_id)`. Height derived from hash, clamped to zone range. |

**Per-cell generation logic:**
1. Convert cell world coords to fractional position.
2. Query Voronoi → zone affinity → blended `ZoneParams`.
3. `street::layout_block` determines if the cell is a street (height = 0).
4. If not a street, `building::assign_building` assigns height and palette.
5. Compute `InteriorId` via hash for every built cell (interior module is
   still a stub at this point).

**Tests:**
- Same `(cx, cy, seed)` always produces the same `ChunkBuffer`.
- Adjacent chunks agree at their shared edge cells (deterministic overlap).
- Downtown cells tend toward taller buildings than residential cells
   (statistical property check over many chunks).

**Exit criteria:** deterministic chunk output, cross-chunk edge consistency.

---

### Milestone 4 — Cache & Engine — ✅ DONE

**Goal:** `WorldEngine` manages chunks with an LRU cache and exposes the
public method surface.

| File | Deliverable |
|---|---|
| `src/cache.rs` | `LruCache<ChunkId, ChunkBuffer>`. Evicts chunks whose Chebyshev distance from the current center exceeds `draw_distance`. |
| `src/engine.rs` | `WorldEngine` struct. Methods: `generate_chunk`, `get_zone_affinity`, `set_draw_distance`, `evict_distant_chunks`. |

**Tests:**
- Generating a chunk that already exists returns the cached copy (no recomputation).
- After moving the center and generating new chunks, chunks beyond `draw_distance + margin` are evicted.
- Memory footprint stays bounded over a simulated 1000-step walk.

**Exit criteria:** cache eviction verified, bounded memory in simulation.

---

### Milestone 5 — FFI & Binary Format — ⬜ PENDING

**Goal:** the engine is callable from C with a `repr(C)` binary protocol.

| File | Deliverable |
|---|---|
| `src/ffi.rs` | `extern "C"` functions: `urbix_engine_create`, `urbix_engine_destroy`, `urbix_generate_chunk`, `urbix_chunk_free`, `urbix_get_zone`, `urbix_set_draw_distance`, `urbix_set_chunk_size`. |
| `build.rs` | Runs `cbindgen` to generate `include/urbix.h`. |
| `include/urbix.h` | Generated C header, checked into the repo. |

**Memory contract:**
- `urbix_generate_chunk` allocates a buffer; caller must call `urbix_chunk_free`
  on the returned `UrbixChunkBuffer` to release it.
- Engine handle is opaque; only accessed through FFI functions.

**Tests:**
- C integration test: `examples/basic_usage.c` compiles with `cc`, links
  against the engine, generates a chunk, reads cell data, and frees it.
- Fuzz test: random sequence of create → generate → destroy, no leaks
  (checked via Valgrind or AddressSanitizer).

**Exit criteria:** C example compiles and runs, no memory errors.

---

### Milestone 6 — Interior Hooks — ⬜ PENDING

**Goal:** wire the interior subsystem so every built cell has a stable
`InteriorId`, and the interface for future interior generation exists.

| File | Deliverable |
|---|---|
| `src/interior.rs` | `InteriorState` trait: `fn generate(id: InteriorId, seed: u64) -> Self`. Stub implementation returns a placeholder state. `InteriorCache` struct (parallel to chunk cache but keyed by `InteriorId`). |
| `src/data.rs` | `InteriorId` already present from Milestone 1; ensure it is populated in every `Cell` during chunk generation. |

**Design constraint:** interiors are a separate mini-world (own small grid,
independent of outdoor chunks). The `InteriorState` trait defines the hook
surface; the actual generation logic is deferred.

**Tests:**
- `InteriorId` is deterministic: same cell coords + same seed → same id.
- `generate_interior` on the stub returns a non-null placeholder.
- Two calls with different seeds produce different placeholder states.

**Exit criteria:** trait compiles, stub passes tests, no regressions on
existing chunk tests.

---

### Milestone 7 — CLI, Docs & Benchmarks — ⬜ PENDING

**Goal:** a command-line tool that generates and dumps city data, full
documentation, and performance baselines.

| File | Deliverable |
|---|---|
| `src/main.rs` | CLI with flags: `--seed`, `--cx`, `--cy`, `--radius`, `--chunk-size`, `--format bin\|json`. Uses `clap` for argument parsing. |
| `README.md` | Expanded: project description, build instructions, quickstart, CLI usage, API overview. |
| `CHANGELOG.md` | Initial `v0.1.0` entry following Keep a Changelog format. |
| `docs/world_generation.md` | Deep write-up of Voronoi regions, fuzzy borders, chunk generation algorithm. |
| `docs/api.md` | C API reference with function signatures, memory contract, examples. |
| `benches/chunk_gen.rs` | Criterion benchmarks: single chunk generation, 100-chunk sweep, cache hit vs. miss. |

**Tests:**
- CLI integration test: generate a chunk, verify output file matches expected
  binary layout.
- Doc tests compile and run.

**Exit criteria:** CLI works end-to-end, benchmarks recorded, all docs complete.

---

## 8. Future Extensions (explicitly deferred)

These are deliberately out of scope for the initial build but are designed for
by the current architecture. Each is listed with the hook already in place.

### 8.1 Interior generation & rendering
- `interior.rs` already exposes the `InteriorState` trait and computes a stable
  `InteriorId` for every built cell.
- Future work: implement real room layouts (grid, corridors, doors), per-room
  fog/palette, furniture placement, and an enter/exit teleport API.
- The design treats interiors as a separate mini-world with its own cache,
  so they never interact with outdoor chunk eviction.

### 8.2 Rendering / visualization consumer
- This engine is intentionally render-agnostic; the first consumer could be a
  standalone debug renderer (e.g. a minimal OpenGL/WebGPU viewer) or a "make it
  pretty" showcase project.
- The FFI binary protocol (`UrbixChunkBuffer`) is designed so a renderer can
  stream chunks as the player walks.

### 8.3 Roads & navigation network
- Streets currently exist only as `height = 0` cells with block layouts.
- Future: build a real road graph (nodes + edges) over the street grid,
  including intersections, avenue widths, and junctions, enabling navigation,
  traffic, and pathfinding.
- Would be a new `road_net.rs` module consuming the same Voronoi/cell data.

### 8.4 Terrain & elevation
- The city currently lives on a flat plane.
- Future: Voronoi-based noise height field with waterfront/coastal zones, hills
  that affect building placement and road grades. Likely in `terrain.rs`
  blended into chunk generation before buildings are placed.

### 8.5 Time of day & weather (data only)
- Provide a per-cell or per-chunk environment struct (ambient light, sky,
  weather effects) as pure data, letting the consumer decide how to render it.

### 8.6 Dynamic / mutable city
- Currently everything is deterministic and immutable once generated.
- Future: allow directed mutations (a player building a structure) via an
  overlay that separates authored edits from the procedural base, so edits
  survive chunk regeneration.

### 8.7 Multi-language bindings
- The C FFI enables bindings to any language with C interop (Python via ctypes,
  C++, C#, Godot via GDExtension, Unity via unsafe extern, WebAssembly via
  wasm-bindgen). Official bindings for a chosen target can be added as separate
  repositories or tools.

### 8.8 Distributed / server-side generation
- Generation is deterministic and dependency-free, so it parallelizes trivially.
- Future: a server that pre-generates chunks on demand for clients, or a
  worker-pool that parallelizes chunk generation across cores (rayon) — the
  per-chunk independence already guarantees safety.