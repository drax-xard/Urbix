# World Generation — Urbix

This document describes how Urbix turns a single `seed: u64` into an infinite,
deterministic city. It mirrors `Urbix_Project.md §3–§4` but dives into the
actual generation pipeline as implemented in `src/`.

Scale canon (Milestone 11): **1 cell = 4 m**. Block sizes are in cells;
multiply by 4 for metres (downtown 11 → 44 m permeable blocks).

## 1. Overview

```
seed
 │
 ▼
Voronoi sites (immutable, 24–48 points) ──►  continuous zone-affinity field
 │                                            + per-district frame (angle/warp)
 │                                            + CBD peak factor
 ▼
per-cell: world_x = cx*CS + lx , world_z = cy*CS + ly  (i64, §1.1)
 │
 ▼
Voronoi query → [f32;5] affinity ──► snapped grid + blended heights
  ──► street? (local/arterial, plaza) ──► sidewalk? ──► lot → building
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
unit-sum, and bisector-sweep continuity — plus the Milestone 11 district
rules: CBD anchor (site nearest the origin is Downtown), adjacency buffer
(Industrial re-tagged Commercial when its nearest neighbour is Residential),
quantized district frames, and the `1.0–1.5` CBD peak factor. Milestone 12
adds two global diagonal boulevards per diagram (an X pair 90° apart, defined
in world coordinates so they cross chunks and districts seamlessly).

## 3. Chunk Layer (`src/chunk.rs`)

A chunk is `chunk_size × chunk_size` cells (default 32). `generate_chunk(cx,
cy, &config, &voronoi) -> ChunkBuffer` walks `local_x/y` in row-major order:

1. `world_x = i64(cx)*CS + local_x`, `world_z = i64(cy)*CS + local_y`.
2. `affinity = voronoi.query(world_x as f64, world_z as f64)`.
3. `params = config.blended_zone_params(&affinity)` (`src/config.rs:337`) —
   heights/density/palette blended; `block_size`/`arterial_every` snapped from
   the dominant zone so transition bands never average grid periods.
4. `frame = voronoi.district_frame_for(...)` — the district's quantized
   rotation (7 angles) + two-octave sine warp (`src/lot.rs`).
5. `info = street::street_info(world_x, world_z, &params, &frame, diags, seam, seed)`
   (`src/street.rs`) — 1-cell local streets plus 2-cell arterials every
   `arterial_every` streets (`IS_ARTERIAL`), two global diagonal boulevards
   (arterial, world-space lines), district-seam parkways (cells within ~1
   cell of a Voronoi bisector pave as arterial avenues both grids tee into),
   and hashed dropout: ~8% of local stretches vanish between arterials
   (superblocks), reopening as `IS_GREENWAY` linear parks 60% of the time.
   Intersections in Downtown/Commercial become `IS_PLAZA` on a 2% hash (15%
   where a diagonal crosses the grid).
6. Sidewalk ring: a non-street, non-green cell abutting any street cell (same
   full query on 4-neighbours) becomes `IS_SIDEWALK` — paved, no-build.
7. Lots: `lot::block_loc` + `lot::lot_slot` split the block interior into a
   2-D pack of up to 9 lots (`domain::LOT_SPLIT`).
   `building::assign_building` derives one height/palette per lot
   (block-noise clumping, CBD boost, corner bonus, landmark 1.5×) with ±10%
   per-cell jitter (`domain::LOT_HEIGHT`). Special blocks rewrite the
   program: civic `Plaza` squares, `Market` shed rows (≤ 10 u), `TowerPark`
   single towers (×1.35) in green blocks (`domain::SPECIAL`).
8. If `height > 0`, `interior_id = interior_id_for_lot(block, slot, seed)`
   (`src/chunk.rs:225`, `domain::INTERIOR`) — one key per lot, shared by
   every cell in it.

`IS_PARK` is set on unpaved cells where `Park` affinity dominates, plus a
hashed 15% of Residential blocks (courtyard gardens). Paved cells
(street/sidewalk) never carry it.

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
