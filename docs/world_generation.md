# World Generation — Urbix

This document describes how Urbix turns a single `seed: u64` into an infinite,
deterministic city. It mirrors `Urbix_Project.md §3–§4` but dives into the
actual generation pipeline as implemented in `src/`.

## 1. Overview

```
seed
 │
 ▼
Voronoi sites (immutable, 24–48 points) ──►  continuous zone-affinity field
 │
 ▼
per-cell: world_x = cx*CS + lx , world_z = cy*CS + ly  (i64, §1.1)
 │
 ▼
Voronoi query → [f32;5] affinity ──► zone_params blend ──► street? ──► building
 │
 ▼
Cell { height, zone_affinity, palette_id, flags, interior_id }  (40 B)
 │
 ▼
ChunkBuffer { ChunkHeader (32 B) + cells } ──► ChunkCache (LRU, Chebyshev)
```

All steps are pure functions of `(world_x, world_z, seed, domain)` via
`hash::hash_coords` (`src/hash.rs:85`). No global RNG, no cross-chunk writes.

### 1.1 Wide coordinates

`world_x / world_z` are `i64`. `cx: i32 * chunk_size: i64 + local: i64` keeps
the city correct past `|cx| >= 2^26` where the old `i32` path overflowed.
Values inside `i32` hash byte-identically to the legacy `i32` formula
(sign-extension), so existing cities are unchanged (`src/hash.rs:62`).

## 2. Voronoi Region Layer (`src/region.rs`)

1. `VoronoiDiagram::generate(seed, site_count)` hashes `seed` to produce
   `site_count` (24–48) points uniformly in `±10_000` world units.
2. Each site is tagged with a `ZoneType` (`Downtown`, `Residential`,
   `Commercial`, `Industrial`, `Park`) via weighted random from the same hash
   stream (`domain::SITE_ZONE`).
3. `query(x, z)` blends **all** sites with Shepard inverse-distance weighting:
   `w_i = 1 / (d_i^p + eps)` (tiny `eps` avoids singularity on-site), summed
   per zone and normalised. Because every `w_i` is continuous in `(x,z)`, the
   affinity vector is continuous everywhere — no snapping at triple points, no
   hard edges. Deep inside a cell the nearest site dominates; near borders the
   blend trades off smoothly. This replaced the earlier nearest-two + smoothstep
   blend that snapped at triple points (`CHANGELOG.md 0.3.0`).

Diagnostics covered in `src/region.rs` tests: determinism, near-1.0 at a site,
unit-sum, and bisector-sweep continuity.

## 3. Chunk Layer (`src/chunk.rs`)

A chunk is `chunk_size × chunk_size` cells (default 32). `generate_chunk(cx,
cy, &config, &voronoi) -> ChunkBuffer` walks `local_x/y` in row-major order:

1. `world_x = i64(cx)*CS + local_x`, `world_z = i64(cy)*CS + local_y`.
2. `affinity = voronoi.query(world_x as f64, world_z as f64)`.
3. `params = zone_params(&affinity)` (`src/zones.rs:155`) — density,
   `height_min/max`, `block_size`, `palette_count` blended by affinity.
4. `flags = street::layout_block(world_x, world_z, &params)` (`src/street.rs`) —
   per-zone street grid via `rem_euclid` on absolute coords (sign-stable,
   cross-chunk consistent). Streets have `height = 0`.
5. If not a street, `building::assign_building(world_x, world_z, &params, seed)`
   (`src/building.rs:53`) derives height clamped to the zone band and a
   `palette_id` via `hash_coords(..., domain::HEIGHT/PALETTE)`. A density roll
   (`domain::DENSITY`) may leave an empty lot (`height = 0`).
6. If `height > 0`, `interior_id = hash_coords(world_x, world_z, seed,
   domain::INTERIOR)` (`src/chunk.rs:127`) — stable for M6 interiors.

`IS_PARK` is set on non-street cells where `Park` affinity dominates
(`dominant_zone` argmax, tie → lower index, matching `examples/viz.rs`).

Cells are packed into `ChunkBuffer` (`src/data.rs:146`): header fields written
at `offset_of!` offsets into a zeroed `Vec<u8>` so implicit padding stays `0`
and deterministic.

## 4. Cache & Engine (`src/cache.rs`, `src/engine.rs`)

`WorldEngine` owns `WorldConfig`, the immutable `VoronoiDiagram`, a
`ChunkCache` centered at `(0,0)` with `draw_distance` (default 8), and a
`generated_count` metric.

* `generate_chunk(cx,cy)` — hit: `cache.get` touches recency and returns
  `clone`; miss: `chunk::generate_chunk`, `cache.insert`, `evict_distant_chunks`.
* `set_draw_distance(dd)` / `set_center(cx,cy)` — deferred eviction.
* `set_chunk_size(cs)` — asserts `cs > 0`, updates `config`, `cache.clear()` so
  stale-size buffers cannot mix.

`ChunkCache` (`src/cache.rs:40`) is `HashMap<ChunkId, Entry { value, last_used }>`
with a monotonic `tick`. Eviction: retain where `chebyshev(id,center) <= dd`
(`src/cache.rs:209` computed in `i64` to avoid `i32::MIN..MAX` overflow), then
if `len > capacity` drop LRU excess. `clear()` is used on chunk-size changes.
A 1000-step walk test proves bounded memory (`src/engine.rs:292`).

`InteriorCache` (`src/interior.rs:123`) mirrors this but keyed by `InteriorId`
and purely capacity-based (no draw distance — interiors are a separate mini-world).

## 5. Determinism Invariants

* Same `(world_x, world_z, seed, domain)` → same `hash_coords` output on any
  run/platform (`src/hash.rs:85`).
* Same `(cx,cy,seed,config,voronoi)` → byte-identical `ChunkBuffer`
  (`src/chunk.rs:179`).
* Adjacent chunks agree on shared edge cells because every cell queries the
  same continuous Voronoi field and absolute coords (`src/chunk.rs:238`).

## 6. Wire Format

`ChunkBuffer` is header (32 B) + `cell_count * Cell` (40 B, `src/data.rs:107`)
with no inter-record padding. Consumers read `ChunkHeader` then cast
`data + sizeof(header)` to `UrbixCell[N]`. See `docs/api.md` and
`Urbix_Project.md §2.3`.

## 7. Future Hooks

* `InteriorState` / `PlaceholderInteriorState` (`src/interior.rs:1`) — per-built-cell
  deterministic interiors ( §4.4 ).
* Street graph, terrain elevation, time/weather — deferred per §8 but designed
  to slot into the per-cell pipeline without breaking determinism.
