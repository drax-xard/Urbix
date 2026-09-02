# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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
