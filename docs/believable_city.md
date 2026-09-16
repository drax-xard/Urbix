# Believable City Plan — Walkability-First, Varied Fabric (Milestone 11)

Status: ✅ **DONE** (released in 0.11.0). This document was the build
specification; the implementation follows it as built. Notes on as-built
deviations are marked [AS-BUILT] inline.

This document is the buildable specification for Milestone 11. It refines the
report findings into file-by-file deliverables under the locked decisions:

1. **Ground-level walkability first**, aerial skyline second.
2. **Varied fabric** over grid-iron (per-district rotation/warp + hierarchy,
   intentional seam jogs where fabrics meet).
3. **Scale canon: 1 cell = 4 m.**
4. **`CellFlags` may grow** (`IS_ARTERIAL` / `IS_PLAZA` / `IS_SIDEWALK`).
5. **Arterial spacing is per-zone** (`ZoneParams.arterial_every`), not global.

Parent design: `Urbix_Project.md` §2–§4, §7 (M1–M10 done), §8.3/§8.4 hooks.
Pipeline detail: `docs/world_generation.md`. FFI contract: `docs/api.md`.

## 1. Goals / non-goals

Goals:

* Continuous street walls at eye level (no single-cell sawtooth towers).
* Permeable, short blocks downtown; legible arterials, plazas, landmarks.
* Districts with distinct orientation/warp so adjacent fabrics visibly differ.
* Same hard invariants: pure `hash(x, y, seed, domain)` (`src/hash.rs:75`),
  no cross-chunk writes, LRU-bounded memory (`src/cache.rs:40`), FFI-first.

Non-goals for M11:

* No road-graph pathfinding/traffic (stays a §8.3 pure query, not a stored graph).
* No terrain elevation/water (stays §8.4 deferred; mask hooks only).
* No `Cell` size change (`Cell` stays 40 B, `src/data.rs:134`). Flags gain bits only.

## 2. Scale canon (1 cell = 4 m)

| Quantity | Cells | Metres | Note |
|---|---|---|---|
| Cell | 1 | 4 | single `Cell`, `src/data.rs:116` |
| Chunk (default 32) | 32×32 | 128×128 | `WorldConfig.chunk_size`, `src/config.rs:122` |
| Downtown block (new) | 10–12 | 40–48 | was 4 (~16 m, unbuildable); fixes `src/chunk.rs:149` 7×7 floor |
| Residential block | 10 | 40 | courtyard/garden interior |
| Commercial block | 9 | 36 | service alley + shopfront |
| Industrial block | 14 | 56 | yard, longer but still walkable edge |
| Park block | 18 | 72 | green mass, not speckle |
| Arterial spacing | `K × block` | e.g. 4×40 = 160 m downtown | per-zone `K`, see §5 |
| Plaza | 3×3 | 12×12 | hashed intersections only |

Why this matters: the old Downtown `block_size 4` (`src/config.rs:133`)
left a 3×3 buildable interior, which is why interiors needed an artificial
7×7 floor. M11 removes the root cause instead of patching it.

## 3. Hierarchy (the one mental model)

```
district = nearest Voronoi site id        (piecewise constant, NOT blended)
block_origin = quantize(rotated+warped world, block_size)
lot_id = hash(block_origin, lot_slot, seed)   (2–6 street-facing lots/block)
cell → (district, block_origin, lot_id) → street? → lot values → cell jitter
```

* District frame (angle/warp) comes from the **nearest site id** so it is
  constant inside a district and jumps cleanly at borders (T-junctions and
  terminated vistas are the desired varied-fabric effect).
* Shepard affinity (`src/region.rs:160`) still drives heights/density blending.
* `block_size` snaps to the dominant zone; it is never averaged (replaces the
  rounded mean in `src/zones.rs:189` that produced hybrid 7/9/10-cell grids).

## 4. Subtasks

### 11.1 Lots + street wall (`src/lot.rs` new, `src/building.rs`, `src/chunk.rs`)

* `block_origin_for(world, block_size, frame)` via `div_euclid` on the
  rotated/warped coords; `lot_id_for(block_origin, slot, seed)`.
* `subdivide_block`: split the block interior along its long axis into 2–6
  hashed lots facing the nearest street. Reuses `door_side_for`
  (`src/chunk.rs:176`) so doors already face streets.
* `assign_building` takes `(lot_id, block_noise)`:
  one height/palette/density per lot, per-cell jitter ±10% only.
  Density roll moves to lot scale (ends salt-and-pepper vacancy).
* Street-wall rules: minimum frontage height per lot row, no single-cell gaps,
  corner lots +15–30% height with distinct palette index.
* `interior_id = hash(lot_id)` so a lot shares one interior key (today each
  cell has its own key via `src/chunk.rs:128`).

### 11.2 Varied-fabric streets (`src/region.rs`, `src/street.rs`, `src/data.rs`)

* `district_frame(site_id) -> { angle, warp_amp, warp_len }` from the
  `SITE_X/Y` stream (`src/region.rs:110`). Quantize angles to
  `{0°, ±12°, ±24°}` and warp to `0.5–1.5` cells so seams jog without breaking.
* `street.rs` replacement: transform
  `local = Rot(-angle) * world + warp(world)`, then:
  local streets `x % block == 0 || y % block == 0` (1 cell),
  arterials every `K`-th street (`K = ZoneParams.arterial_every`), 2 cells wide.
* New flags in `src/data.rs:80` (bits only, size unchanged):
  `IS_ARTERIAL (1<<2)`, `IS_PLAZA (1<<3)`, `IS_SIDEWALK (1<<4)`.
* Per-chunk cache of `district_frame` (sites are only 24–48) to avoid a
  per-cell nearest-search + `sin` blowup.

### 11.3 Block anatomy for walkers (`src/chunk.rs`, `src/street.rs`)

* Perimeter lots face the street (active frontage; Commercial gets shopfront
  depth rule). Interiors by zone: Residential garden/courtyard (`IS_PARK`),
  Commercial rear alley, Industrial yard, Downtown lobby + plaza apron.
* 1-cell `IS_SIDEWALK` ring inside the street edge (curb read, building face
  off asphalt).
* Plazas: ~2% of Downtown/Commercial intersections via `PLAZA` domain hash →
  3×3 `IS_PLAZA`, no-build. Adjacent landmark lot gets 1.5–2× height outlier
  (`LANDMARK` domain) — the Lynch node + landmark pair.

### 11.4 District v2 + skyline peak (`src/region.rs`, `src/config.rs`)

* At `generate_with_config` time only (zero per-cell cost):
  adjacency penalty (re-roll `Industrial|Residential` abut),
  pull `Commercial` toward the arterial lattice,
  force one CBD cluster near the origin.
* Height multiplier `f(dist to nearest Downtown site)` for peak-and-taper
  (keeps the `downtown_taller_than_residential` invariant in
  `src/chunk.rs:353` but adds spatial structure).

### 11.5 Config, FFI, viz, metrics

* `ZoneParams` gains `arterial_every: u8` (every K-th street is arterial,
  `K=0` treated as “no arterials” → clamped to a sane default in code, never
  panics across FFI). `WorldConfig` size grows → MINOR bump (0.11.0),
  `is_valid` (`src/config.rs:196`) guards `1..=16`, cbindgen allow-list +
  `include/urbix.h` regen, `urbix.toml.example` / `urbix.json.example` regen,
  serde defaults keep old files parsing.
* `viz.rs` (`examples/viz.rs:110`) + `interactive.rs`: add `lots` and `walk`
  modes (lot boundaries, arterials, plazas, sidewalks).
* Walkability metrics (new `examples/walkability.rs` or viz flag):
  mean block perimeter, 4-way vs T-junction ratio, uninterrupted street-wall
  length, plaza/landmark density per km². Must improve over `main` baseline.

## 5. File deliverables

| File | Change |
|---|---|
| `src/hash.rs:35` | New domains `LOT_SPLIT:50`, `LOT_HEIGHT:51`, `BLOCK_NOISE:52`, `ORIENTATION:53`, `LANDMARK:54`, `PLAZA:55` |
| `src/lot.rs` (new) | `DistrictFrame`, `block_origin_for`, `lot_id_for`, `subdivide_block`; re-export in `src/lib.rs` |
| `src/region.rs:110,160` | `district_frame()` (nearest-site); CBD/adjacency rules at generation; keep Shepard query for affinity |
| `src/street.rs:51` | Rotated/warped hierarchy (local + 2-cell arterials); seam-jog behaviour documented |
| `src/zones.rs:80,189` | `ZoneParams.arterial_every`; document block_size snap (no averaging) |
| `src/building.rs:53` | Per-lot height/palette/density + cell jitter; corner bonus |
| `src/chunk.rs:59,128,149,176` | Lot-context orchestration; lot-keyed `interior_id`; remove 7×7 patch once blocks grow; keep `door_side_for` |
| `src/data.rs:80` | 3 new `CellFlags` bits; keep 40 B / 8-align asserts (`src/data.rs:134`) |
| `src/config.rs:86,122,196` | Defaults re-tune (§2 table) + `arterial_every`; `is_valid`; file round-trip |
| `src/ffi.rs`, `build.rs`, `cbindgen.toml`, `include/urbix.h` | Export new flags + `ZoneParams` size; regen header |
| `urbix.toml.example`, `urbix.json.example` | New fields with 4 m-scale comments |
| `examples/viz.rs`, `interactive.rs`, `examples.md` | `lots`/`walk` modes; walkability metrics |
| `docs/world_generation.md`, `docs/api.md` | Update pipeline + flags + scale canon |

## 6. Determinism and cross-chunk rules

* Everything stays `(world_x, world_z, seed, domain)`-pure; no stored graph.
* Block/lot derivation uses absolute coords + `div_euclid`/`rem_euclid`
  (same sign-stability as `src/street.rs:51` and `src/chunk.rs:76`).
* Orientation varies per district id only, never continuously, so chunk edges
  agree: two chunks querying the same world point compute the same frame.
* Old seeds change output (expected MINOR break); no byte-identical guarantee.

## 7. Compatibility

* Rust API: `assign_building` signature changes (lot context) — MINOR.
* C ABI: `Cell` layout unchanged (flags bits are additive); `ZoneParams` /
  `WorldConfig` grow → recompile header, bump to `0.11.0`, note in
  `CHANGELOG.md` + `Urbix_Project.md` §7.
* Config files: serde defaults fill `arterial_every` so pre-11 files parse
  and reproduce new defaults only after re-save.

## 8. Tests and exit criteria

* Unit: lot determinism across chunks; same-lot height/palette share;
  arterial spacing per zone; rotation determinism; seam chunk-consistency;
  sidewalk ring present; plaza no-build + landmark outlier exists;
  `Cell` 40 B asserts green; config round-trip with old files.
* Property: street-wall continuity (mean uninterrupted frontage) up vs `main`;
  block perimeters within §2 bands; 1000-step walk still bounded
  (`src/engine.rs` walk test pattern).
* Manual: `viz --mode walk` + `interactive` click-through — doors face
  streets, no floating single-cell towers, plazas break the grid.
* Perf: `cargo bench --bench chunk_gen` — per-chunk frame cache keeps
  regression < 2× vs `main`; document result.

## 9. Risks

* Per-cell `sin` + nearest-search cost → mitigate with per-chunk frame cache.
* Seam stubs reading as broken rather than varied → mitigated by quantized
  angles + small warp (§4, 11.2).
* Re-tuned blocks change downtown density feel → validate via walk metrics
  before locking defaults.

## 10. Sequencing

Build in order; each step is testable alone: 11.1 → 11.2 → 11.3 → 11.4 → 11.5.
Do not start terrain/water (§8.4) until arterials can follow grades.

## References

* Pipeline: `docs/world_generation.md:1`, `src/chunk.rs:59`, `src/street.rs`,
  `src/building.rs`, `src/zones.rs:157`, `src/region.rs:160`.
* Wire/FFI: `docs/api.md:1`, `src/data.rs:116`, `src/ffi.rs`, `include/urbix.h`.
* Interiors bridge: `docs/interiors.md:182`, `src/chunk.rs:interior_context_for`.
* Status: `Urbix_Project.md` §7 M11 (done), `README.md` Status.

## 11. As-built notes (0.11.0)

* [AS-BUILT] `ZoneParams` gained `arterial_every` without growing: 3×`f32` +
  3×`u8` still pads to 16 B, so the C struct is source-compatible (recompile
  the header; no layout migration). `WorldConfig` size is likewise unchanged.
* [AS-BUILT] Block anatomy stays implicit: courtyard gardens appear as hashed
  garden blocks (15% of Residential) plus clumped vacancy, not carved
  per-block courtyards/alleys — perimeter lots already face streets, so the
  walkability read lands without a 2-D lot packer. Explicit courtyard carving
  is deferred to a follow-up.
* [AS-BUILT] `viz` gained a `walk` pedestrian mode; per-lot boundary
  rendering (`lots` mode) was dropped — lots are not on the wire, and
  recomputing them in the example would duplicate the pipeline. The
  `walkability` example gates the acceptance metrics instead.
* [AS-BUILT] `street::layout_block` and `building::assign_building`
  signatures changed (framed grid; lot context) — a Rust MINOR break covered
  by the 0.11.0 bump. The old per-cell behaviour has no fallback flag;
  pre-11 seeds regenerate with the new fabric.
