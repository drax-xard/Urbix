# Urbix

A platform-agnostic and language-agnostic modular engine for generating an
explorable, infinite procedural city that can be used/called from other engines
either via CLI or API.

This is a Rust crate (library) that deterministically generates an unbounded
city in **chunks on demand**. It exposes its core data types through a C FFI
surface, so renderers, game engines, or other tools in any language can consume
the generated city without depending on Rust.

## Objectives

The full design lives in [`Urbix_Project.md`](Urbix_Project.md); in short:

- **A convincing, varied skyline.** Distinct urban regions — skyscraper business
  districts, tranquil residential areas, busy commercial zones, dirty industrial
  zones, and green parks — each with its own density, building-height
  distribution, and color scheme.
- **Infinite world, deterministic generation.** The city is generated per-chunk
  on demand, so it is effectively unbounded while remaining reproducible from a
  seed.
- **High-performance, bounded memory.** Chunks are LRU-cached and evicted once
  they fall beyond the current draw distance, so infinite generation never
  grows unbounded in RAM.
- **Extensible and modular.** A clean module boundary (hashing, zones, regions,
  streets, buildings, caching, engine) lets capabilities be changed or expanded
  independently.
- **Truly engine-agnostic.** Urbix only *generates* the city; rendering, UI,
  gameplay, and everything else is left to the consuming software.

## Status

Tracked milestone-by-milestone in `Urbix_Project.md` §7 (✅ done / ⬜ pending).
Current version: `0.6.0` (see `CHANGELOG.md`).

| Milestone | Status |
|---|---|
| M1 — Data layer & hashing | ✅ Done (0.2.0) |
| M2 — Voronoi region layer | ✅ Done |
| M3 — Chunk generation core loop (streets + buildings) | ✅ Done (0.3.0) |
| M4 — Cache & engine facade | ✅ Done (0.4.0) |
| M5 — FFI & binary wire format | ✅ Done (0.5.0, audit 0.5.1) |
| M6 — Interior hooks | ✅ Done (0.6.0) |
| M7 — CLI, docs & benchmarks | ⬜ Pending |

Each chunk is produced by: querying the continuous Voronoi zone field at the
cell's absolute world coordinates → resolving blended zone parameters → laying
out the street grid → placing buildings (height, facade palette, interior key),
then packing the results into flat `#[repr(C)]` cell records.

## Design choices

- **Deterministic from a seed.** Everything derives from a seeded hash
  `hash(x, y, seed, domain) -> u64`. There is no global RNG and no cross-chunk
  write dependency, so the same seed always reproduces the same city, and
  adjacent chunks agree at their shared edges.
- **Fuzzy Voronoi districts.** A fixed set of seed-derived Voronoi sites
  (24–48) are mapped to the five zone types and queried *continuously* for zone
  affinity, rather than stored as a per-chunk static map — giving soft, blended
  borders between districts.
- **Wide (i64) world coordinates.** Per-cell generation and hashing operate on
  `i64` so an effectively infinite city never overflows the old `i32` range.
  Coordinates within `i32` produce byte-identical output to earlier versions.
- **FFI-first / language-agnostic.** All public data types are `#[repr(C)]`
  and match a fixed on-wire layout (`Cell` = 40 B, header = 32 B). The C header
  (`include/urbix.h`) is auto-generated from `src/ffi.rs` via `cbindgen` in
  `build.rs` (checked in, best-effort so offline builds keep working), and the
  crate emits both `staticlib` (`liburbix.a`) and `cdylib` for C consumers.
  `urbix_generate_chunk` transfers buffer ownership to the caller (must be freed
  only via `urbix_chunk_free`); the engine handle is opaque. Compat shims
  `URBIX_FLAG_STREET`/`URBIX_FLAG_PARK` are preserved.
- **Bounded memory.** `ChunkCache` keeps chunks keyed by `ChunkId` and evicts
  by Chebyshev distance from the current center, with an optional hard capacity.
- **Dependency-free library.** The core crate has no runtime dependencies;
  the only external crate (`image`) is dev-only and used by the visualizer.

## Visualizer

A small example tool renders a generated region to an image so the engine's
output can be inspected by eye:

```sh
cargo run --release --example viz -- \
  --seed 445566 --center-cx 0 --center-cy 0 --extent 8 --chunk-size 32 --out out.png
```

It paints one pixel per cell and supports two colouring modes: `hybrid`
(per-district zone hue, brightened by building height, roads drawn as streets)
and `affinity` (flat dominant-zone colour). It writes both a dependency-free
P6 PPM (`.ppm`) and a PNG (`.png`). Run with `--help` for the full flag list.

## Usage

The library is in early development (currently `0.6.0`); the public surface is
`WorldEngine` in Rust and the C ABI in `include/urbix.h` (see
`Urbix_Project.md` §2.3-2.4 for the wire format):

```c
#include "urbix.h"
UrbixEngine *e = urbix_engine_create(445566);
UrbixChunkBuffer buf = urbix_generate_chunk(e, 0, 0);
// buf.data is ChunkHeader (32 B) + cell_count * UrbixCell (40 B)
urbix_chunk_free(buf);
urbix_engine_destroy(e);
```

`examples/basic_usage.c` is the minimal C consumer (compile with `cc -I include`
and link against `target/{debug,release}/liburbix.a`). `examples/viz.rs` is the
visualizer below; `examples/cli_demo.rs` remains a stub for the walk-grid demo.

## Verification

```sh
cargo build --all-targets
cargo test
cargo clippy --all-targets
cargo fmt --check
```

Note: Rust here is installed via rustup but not on the default `PATH`; source it
with `. "$HOME/.cargo/env"` first in a fresh shell.
