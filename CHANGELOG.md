# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.0] — 2026-09-04

### Added

- **Interior layout generation** (Milestone 9): the exterior→interior
  bridge that lets a generated interior react to the lot it belongs to.
  - `src/layout.rs`: `InteriorContext` (`#[repr(C)]` snapshot of a built lot's
    zone, blended affinity, height, floor count, footprint, palette, seed; floors
    derived from height via `interior_floor_height`), `Tile` (`#[repr(u8)]` grid
    cell kinds: void/wall/door/core/corridor/room), `Floor` / `InteriorLayout`
    (the generated mini-world), and the per-zone blueprint schema
    `Blueprint`/`BlueprintRoom` — fixed-size `#[repr(C)]`, serde-tunable tables
    defaulting to `blueprint_defaults(zone)`.
  - `interior::generate_layout(id, ctx, blueprint)` — the Milestone 9
    room-placement generator: per storey a sealed wall ring, a hashed
    circulation core, a street-facing entrance `Door` on `ctx.door_side` (with
    its inner cell reserved as corridor), weighted room rolls from the
    blueprint tables placed greedily inside a one-tile navigable margin (rooms
    may hug walls and the core, never each other), corridor fill of leftover
    cells, and one `Door` per room punched onto its facing margin cell.
    `floor_hash` folds the storey index in, so floors vary independently while
    staying reproducible per `(id, floor, seed)`.
  - `DoorSide` (`West/East/North/South`, `#[repr(u8)]`); `InteriorContext`
    gains `door_side`; both serde-serialized for round-trip parity.
  - `chunk::interior_context_for(config, world_x, world_z, cell)` now also
    picks the entrance side: `chunk::door_side_for(world_x, world_z,
    block_size)` selects the street the lot abuts (ties toward
    West/East/North/South).
  - `InteriorState` trait now takes the context: `fn generate(id, ctx) -> Self`
    instead of `fn generate(id, seed)`. `PlaceholderInteriorState` sizes itself
    from the context footprint/floors. `generate_interior` signature updated.
  - `WorldConfig` gains `interior_floor_height`, `interior_max_floors`,
    `interior_blueprints: [Blueprint; ZONE_COUNT]` (serde-defaulted for
    backward compat), plus `blueprint_for(zone)` and
    `interior_context(...)` helpers. New hash domains `LAYOUT_PICK/FLOOR/ROOM/
    ROOM_SIZE/DOOR/FURNITURE`.
  - `chunk::interior_context_for(config, cell)` reconstructs the context for a
    built cell (dominant zone + block footprint), so consumers can regenerate
    any cell's interior from the wire data.

### Changed

- `ZoneType` now derives `Serialize`/`Deserialize` (needed by the context).

### Docs

- `docs/interiors.md` — interior generation & blueprint reference (data model,
  defaults, config override chain, consumption), linked from `README.md`.

## [0.8.0] — 2026-09-03

### Added

- **Modular customization** (Milestone 8): all non-procedural tunables are now
  loaded from text files or runtime parameters. `WorldConfig` extended with
  `voronoi_span`, `shepard_power/epsilon`, `zone_weights`, `zones: [ZoneParams;5]`,
  `zone_hues`, `interior_width/height_range` (all `Serialize`/`Deserialize`,
  `#[repr(C)]` bump to `0.8.0`, C header `WorldConfig` now larger). `Default`
  remains byte-identical to pre-8.0 hardcoded values.
  - `WorldConfig::from_file` / `from_toml_str` / `from_json_str` / `from_str_with_path`
    sniff `TOML` vs `JSON` by extension (fallback TOML→JSON), `is_valid` now guards
    `voronoi_span 100..100_000`, `shepard` ranges, `zone_weights` sum ~1.0,
    per-zone `height_min<=max` etc., interior ranges.
  - `WorldConfig::blended_zone_params`, `zone_params_for`, `hue_for`.
  - `VoronoiDiagram::generate_with_config` uses `config.voronoi_span`/`shepard_*`/
    `zone_weights`; `WorldEngine::with_config` + new `set_config(WorldConfig)`
    (validates, regenerates Voronoi, clears cache).
  - `ZoneParams` now `Serialize`/`Deserialize`; `interior::PlaceholderInteriorState::generate_with_config`.
  - `urbix.toml.example` / `urbix.json.example` full example configs.
  - CLI `--config <path>` (TOML or JSON) with file-as-base, CLI flags
    (`--seed`, `--chunk-size`, `--draw-distance`, `--voronoi-site-count`)
    override file; `Format` enum, radius/chunk-size caps, `--out` file-vs-dir
    validation.
  - FFI `urbix_engine_create_with_config`, `urbix_set_config` (null/invalid → no-op,
    never unwind) and `WorldConfig`/`ZoneParams` exported to `include/urbix.h`
    (ZoneParams injected before `WorldConfig` if cbindgen omits).

### Changed

- **`Cargo.toml`**: added `toml = "0.8"` for `TOML` support.
- **`cbindgen.toml`**: `ZoneParams` before `WorldConfig` in allow-list, `INTERIOR_*`
  constants excluded, `WorldConfig` now exported.
- **`build.rs`**: best-effort header gen now injects `ZoneParams` def if missing and
  keeps asserts inside guard.
- **`Urbix_Project.md`**: Milestone 8 marked ✅ DONE.

## [0.7.1] — 2026-09-03

### Fixed

- **CLI hardening** (`src/main.rs:19`): `Format` is now a typed `ValueEnum`
  (`Bin`/`Json`) instead of `String`; `radius` capped at 64 and checked for
  `i32` overflow (`cx±r`, `cy±r`), `chunk-size` capped at 256 and validated
  via `WorldConfig::is_valid`, `--out` file-vs-directory mismatch now errors
  before generation, and `coords` allocation is `with_capacity` bounded.
- **JSON wire leak** (`src/data.rs:114`): `Cell`/`ChunkHeader` ` _pad` fields
  are `#[serde(skip, default)]` so `json` output no longer leaks internal
  padding bytes and round-trips via `Deserialize` default `0`.
- **Interior determinism** (`src/interior.rs:70`): `PlaceholderInteriorState` now
  splits `id` into low/high 32-bit halves for `hash_coords` to avoid overlapping
  entropy (`id as i64` / `id>>32` overlapped).
- **Benchmark accuracy** (`benches/chunk_gen.rs:53`): `cache/miss` now reuses the
  `VoronoiDiagram` (distinct `cx` per iter) instead of recreating
  `WorldEngine` + Voronoi each iteration, which overstated miss cost.
- **`src/api.rs:28`**: removed `TODO(Milestone 4)` stub; module now re-exports
  `WorldEngine` with a note that the public surface is `engine`/`ffi`.

## [0.7.0] — 2026-09-03

### Added

- **CLI** (Milestone 7): `src/main.rs` now parses `clap` flags `--seed`,
  `--cx`, `--cy`, `--radius`, `--chunk-size`, `--format bin|json`, `--out` and
  drives `WorldEngine`. Single chunk writes one file (`chunk_<cx>_<cy>.bin`
  / `.json` default), `radius` writes a `(2r+1)²` grid to a directory.
  Binary output is raw `ChunkBuffer::as_bytes()`; JSON pretty-prints
  `{header, cells}` via `serde` (`Cell`/`ChunkHeader` now `Serialize`/
  `Deserialize`). Includes unit tests for bin layout + JSON round-trip.
- **Documentation** (Milestone 7): `docs/world_generation.md` (Voronoi Shepard
  blend, `i64` coordinates, chunk pipeline, cache, wire format) and
  `docs/api.md` (C header via `cbindgen`, lifecycle, `UrbixChunkBuffer`
  ownership, zone query, config).
- **Benchmarks** (Milestone 7): `benches/chunk_gen.rs` criterion groups —
  single 32×32 chunk, 100-chunk 10×10 sweep, `WorldEngine` cache hit vs miss
  (`criterion 0.5`).

### Changed

- **`Cargo.toml` dependencies**: added `clap` (derive), `serde`/`serde_json`,
  `anyhow`; `criterion` dev-dependency and `[[bench]]` for `chunk_gen`.
  `README.md` now documents CLI quickstart and `cargo bench --bench chunk_gen`.

## [0.6.0] — 2026-09-03

### Added

- **Interior hooks** (Milestone 6): `src/interior.rs` now defines the hook
  surface for future interior generation. `InteriorState` trait with
  `fn generate(id: InteriorId, seed: u64) -> Self`, stub
  `PlaceholderInteriorState` (deterministic `width`/`height` 6..14, `fog`,
  `palette_id` derived from distinct hash domains `INTERIOR_SIZE_*`/`FOG`/
  `PALETTE`), free function `generate_interior::<S>`, and bounded LRU
  `InteriorCache<S>` (parallel to `ChunkCache` but keyed by `InteriorId` with
  capacity-based eviction). `InteriorId` is already populated in every built
  cell by `chunk.rs:interior_id_for` via `hash::domain::INTERIOR`.
- **Hash domains `INTERIOR_SIZE_W/H`, `INTERIOR_FOG`, `INTERIOR_PALETTE`**
  (`src/hash.rs:30`): four new domain bytes for interior property derivation.

### Changed

- **`cbindgen.toml` export allow-list**: excludes the four new
  `INTERIOR_*` constants so they do not leak into `include/urbix.h`.

## [0.5.1] — 2026-09-03

### Fixed

- **`tests/c_link_run.rs` profile & Linux portability**: replaced the fragile
  `std::env::var("PROFILE")`/`CARGO_TARGET_DIR` lookup with
  `cfg(debug_assertions)` and added `linux` link args (`-ldl -lm -pthread`) so
  the C link-and-run test is robust on clean `cargo test --release` and on
  Linux CI, not just macOS.
- **`include/urbix.h` include-guard**: `build.rs` now injects the
  `_Static_assert` layout checks and the `URBIX_FLAG_STREET`/`URBIX_FLAG_PARK`
  compat shims *inside* `#ifndef URBIX_H` (cbindgen's `trailer` lands after the
  guard). Double-inclusion is now correct and old C consumers keep building.
- **`build.rs` robustness**: uses absolute `crate_dir`-joined paths for
  `cbindgen.toml`/`include/urbix.h`, is idempotent on re-runs, and is
  best-effort — on config/generation failure it emits `cargo:warning` and keeps
  the checked-in header instead of hard-failing (offline/vendored builds).

## [0.5.0] — 2026-09-03

### Added

- **Auto-generated C header** (Milestone 5): `build.rs` now runs `cbindgen`
  against `src/ffi.rs` to regenerate `include/urbix.h`, which is checked into
  the repo. `cbindgen.toml` drives the export allow-list, maps `CellFlags` →
  `uint8_t` and `InteriorId` → `uint64_t`, keeps only the needed `ZONE_COUNT`
  constant, and appends `_Static_assert`s that pin the 32/40-byte layouts.
- **`crate-type = ["lib", "staticlib", "cdylib"]`** (Milestone 5): the crate now
  also emits a C static library (`liburbix.a`) and a dynamic library
  (`liburbix.dylib`/`.so`), so the FFI surface is consumable by any C-compatible
  host, not just Rust.
- **C link-and-run integration test** (`tests/c_link_run.rs`, Milestone 5):
  builds `examples/basic_usage.c`, links it against the `staticlib`, and runs it
  end-to-end, proving the full ABI + ownership contract works from a real C
  consumer (create → generate → read header/cells → validate → free → destroy).
- **FFI fuzz test** (Milestone 5): a stochastic sequence of
  create → generate → free → destroy through the raw FFI, run under the normal
  Rust allocator so double-free, use-after-free, and leaks are caught before they
  escape the boundary.

### Changed

- **Milestone 5 marked DONE** in `Urbix_Project.md` §7, including the updated
  `cbindgen.toml` deliverable, memory-contract notes, and test list.

## [0.4.1] — 2026-09-03

### Added

- **`README.md` project summary**: added sections covering objectives, milestone
  status, design choices, the visualizer tool, and a usage/verification
  walkthrough so the crate's intent and state are clear at a glance.
- **`WorldEngine::set_chunk_size`** and **`ChunkCache::clear`** (pre-Milestone 5):
  the engine can now change the cells-per-side chunk size at runtime. Cached
  buffers were generated at the old size, so they are cleared to prevent stale,
  differently-sized chunks from mixing with new output; the new size applies to
  the next `generate_chunk`. `set_chunk_size(0)` panics.
- **FFI chunk buffer ownership & layout** (pre-Milestone 5): established the
  memory contract that Milestone 5's C surface will rest on.
  - `ChunkBuffer::into_raw_bytes` / `from_raw_bytes` leak and reclaim the packed
    wire bytes across an FFI boundary with no allocator mismatch (returned
    buffers are Rust-allocated and must be freed only via `urbix_chunk_free`).
  - `src/ffi.rs` implements the thin C surface: `urbix_engine_create/destroy`,
    `urbix_generate_chunk`, `urbix_chunk_free`, `urbix_get_zone`,
    `urbix_set_draw_distance`, `urbix_set_chunk_size`. The buffer is returned by
    value as `{ data, len }`; null handles/buffers are tolerated; a zero
    `urbix_set_chunk_size` is a no-op at the C boundary (no panic across FFI).
  - `include/urbix.h` is maintained by hand against `src/ffi.rs` (cbindgen is
    still to be wired in `build.rs`), declaring the `repr(C)` records and
    functions.
  - `examples/basic_usage.c` is a real C consumer (with `_Static_assert`s on
    the 32/40-byte layouts); `tests/ffi_basic.rs` round-trips a chunk from the
    Rust side and compiles the C consumer with `cc` to prove the header stays
    valid.

### Changed

- **`urbix_get_zone` uses `double` world coordinates**: `Urbix_Project.md` §2.4
  now documents `double` (matching the f64 engine signature) instead of the old
  `float` sketch, so the reference doc agrees with the implementation and stays
  precise over the engine's wide coordinate span.

## [0.4.0] — 2026-09-03

### Added

- **`IS_PARK` wire flag now populated**: non-street cells whose Park district
  affinity dominates (argmax of the affinity vector, ties toward lower index)
  receive `CellFlags::IS_PARK`. Previously the flag was documented in the wire
  format but never set, so downstream consumers could not distinguish parkland
  from empty lots. Implementation in `chunk::generate_chunk`; a new `dominant_zone`
  helper resolves the argmax deterministically.
- **Milestone 4 — Cache & Engine**:
  - `src/cache.rs`: `ChunkCache` — distance-based LRU cache for `ChunkBuffer`s
    keyed by `ChunkId`. Evicts chunks whose Chebyshev distance from the
    current center exceeds `draw_distance`. Optional hard capacity cap with
    least-recently-used eviction among candidates. Tracks recency stamps for
    O(1) touch.
  - `src/engine.rs`: `WorldEngine` — stateful facade holding `WorldConfig`,
    `VoronoiDiagram`, and `ChunkCache`. Methods: `generate_chunk(cx, cy)` (cache
    hit → no recomputation; miss → generate, insert, auto-evict),
    `get_zone_affinity(wx, wz)`, `set_draw_distance(dd)`, `set_center(cx, cy)`,
    `evict_distant_chunks()`.
  - 59 lib tests: cache insert/get, distance eviction, negative coords,
    LRU ordering, engine cached reuse, draw-distance control, bounded memory
    over a 1000-step walk, zone-affinity validity, custom config.
- **2D city visualizer** (`examples/viz.rs`): renders a grid of generated
  chunks to an image so the engine's output can be eyeballed. One pixel per
  cell, with two colouring modes — hybrid (per-district zone hue brightened by
  building height, roads drawn as streets) and flat dominant-zone affinity.
  Writes both a dependency-free P6 PPM and a PNG (via a dev-only `image` crate
  with just the `png` feature, so the library stays dependency-free). Flags:
  `--seed`, `--center-cx`, `--center-cy`, `--extent`, `--chunk-size`, `--mode`,
  `--out`.

### Fixed

- **Coordinate overflow (Milestone 3)**: internals that folded world coordinates
  through the pre-hash widen (`hash_coords`/`hash_unit`), per-cell generators
  (`street::layout_block`, `building::assign_building`), and `generate_chunk`'s
  world-cell computation (`cx * chunk_size + local`) now operate on `i64`.
  Previously, an infinite city could overflow `i32` at `|cx| >= 2^26`, panicking
  in debug builds and wrapping in release. Hashes are byte-identical for
  coordinates within the old `i32` range (sign-extension), so no existing city
  data changes.
- **Chunk distance overflow (Milestone 4)**: `ChunkCache::chebyshev` computed
  component differences in `i32`, which underflowed for opposite-sign extremes
  (e.g. `i32::MIN` vs `i32::MAX`). Differences are now computed in `i64`, so the
  true ~4.3B distance is reported instead of a panic/wrap.
- **Malformed-buffer hardening (Milestone 1, pre-FFI)**: `ChunkBuffer`'s
  `header()`/`get_cell()`/`set_cell()` now validate the backing buffer's actual
  byte length against the header's `cell_count` before reading/writing, instead
  of trusting the header blindly. Important groundwork for the Milestone 5 FFI
  surface where buffers may be constructed off-`#[repr(C)]` data.

## [0.3.0] — 2026-09-02

### Changed

- Resolved a latent determinism bug in `ChunkBuffer::new`: the wire header's
  implicit alignment padding (between `_pad` and `seed`) was copied from
  uninitialized stack memory, so two identical buffers could differ in 4 bytes.
  The buffer is now zero-initialised up-front and header fields are written at
  their exact `offset_of!` offsets, keeping every padding byte zero and
  deterministic.
- Consolidated the per-use hash domain constants into `hash::domain` (single
  source shared by `region`, `building`, and `chunk`).
- **Milestone 2 audit fix — continuous zone blending**: replaced the
  nearest-two-site + smoothstep blend in `region::VoronoiDiagram::query` with
  Shepard inverse-distance weighting over all sites. The affinity is now
  continuous everywhere (previously the second-nearest site's identity could
  snap at Voronoi triple points, causing up to a ~0.5 jump in a zone's weight
  over a tiny move). A bisector-sweep continuity test covers the worst case.
- Consolidated the duplicated `ZONE_COUNT` into a single source
  (`data.rs` now re-exports `zones::ZONE_COUNT`).
- Updated `Urbix_Project.md` §3 and §7 to describe the Shepard algorithm.

### Added

- **Milestone 3 — Chunk generation (core loop)**:
  - `src/chunk.rs`: `generate_chunk(cx, cy, config, voronoi) -> ChunkBuffer`
    pipes the full per-cell pipeline — Voronoi affinity query → blended zone
    params → `street::layout_block` → `building::assign_building` → interior
    key — into a packed `ChunkBuffer`. Cell content is keyed on **absolute**
    world coordinates so chunk edges stay continuous.
  - `src/street.rs`: `layout_block(cell_x, cell_y, params) -> CellFlags`
    decides street membership from a per-zone block grid using `rem_euclid` on
    absolute world coords (sign-stable, cross-chunk-consistent).
  - `src/building.rs`: `assign_building(cell_x, cell_y, params, seed)`
    derives height and facade palette from the hash, clamped to the zone's
    range, and applies the zone's density roll for empty lots.
  - `ChunkBuffer` gained typed cell accessors (`get_cell`, `set_cell`,
    `cells`, `cell_count`) via safe unaligned reads/writes.
  - Tests: deterministic regeneration, expected layout, streets at height 0,
    built cells carry interiors, street-flag independence from chunk origin,
    and a spread-sampling "Downtown taller than Residential" check.
- **Milestone 2 — Voronoi region layer**:
  - `src/region.rs`: `VoronoiDiagram` generated deterministically from
    `(seed, site_count)`, with seed-derived site positions over a ±10 000
    span and weighted-random `ZoneType` tagging.
  - Fuzzy `query(world_x, world_z) -> [f32; 5]` zone-affinity via continuous
    Shepard inverse-distance weighting, producing soft, stable district
    borders.
  - Tests for determinism, near-1.0 affinity at a site, unit-sum weights,
    and query continuity (including across site-pair bisectors).

## [0.2.0] — 2026-09-02

### Added

- **Milestone 1 — Data layer & hashing**:
  - `src/hash.rs`: deterministic `hash_coords(x, y, seed, domain) -> u64`
    using a self-contained SplitMix64 finalizer with domain separation.
  - `src/config.rs`: `#[repr(C)]` `WorldConfig` (seed, chunk_size,
    draw_distance, voronoi_site_count) with `Default` and `is_valid`.
  - `src/zones.rs`: `ZoneType` (5 zones), `ZoneParams`, per-zone
    `zone_defaults`, and fuzzy `zone_params(affinity)` blend.
  - `src/data.rs`: `#[repr(C)]` `Cell`, `ChunkHeader`, `ChunkId`,
    `InteriorId`, `CellFlags`, and the owned `ChunkBuffer` with a wire layout
    matching `Urbix_Project.md` §2.3 (compile-time size/offset checks).
  - Unit tests for determinism, layout, blending, and validation.
  - Added `AGENTS.md` with toolchain, verification, and architecture guidance.
- Crate skeleton (`Cargo.toml`, `.gitignore`) and module map under `src/`, each
  file carrying a doc-comment header describing its role in the architecture:
  `api`, `building`, `cache`, `chunk`, `config`, `data`, `engine`, `ffi`,
  `hash`, `interior`, `region`, `street`, `zones`, plus `lib.rs` and `main.rs`.
- Scaffolding directories for tests, benchmarks, examples, and `docs/`, plus a
  C header placeholder at `include/urbix.h` (generated from FFI in Milestone 7).
- Project-level design: filled the architecture, development-plan, and
  future-extensions sections in `Urbix_Project.md`; created `CHANGELOG.md`.

## [0.1.0] — 2026-09-02

### Added

- Initial project document `Urbix_Project.md` describing the objectives,
  architecture, world-generation design, versioning conventions, and
  documentation standards.
- Initial `README.md` with a one-line project description.
- Local git identity configured (`user.name` / `user.email`) for this
  repository.
