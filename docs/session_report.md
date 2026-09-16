# Session report — city believability + interiors M11 → M13

> Working handoff note, not project documentation. Written 2026-09-16 to
> carry context into the next session. Tree is clean at `6c6a6ad`.

## Timeline (all requested, built, verified, committed locally; user pushes)

1. **City-generation audit (plan mode).** Read `Urbix_Project.md`, README,
   CHANGELOG, `docs/*`, and the generators. Reported: cell-soup city, no
   lots; single graph-paper street grid; oversized/smooth districts;
   white-noise density. (`AGENTS.md`'s "all placeholders" note is stale —
   M1–M10 were already done.)
2. **Priorities locked:** ground-level walkability first; varied fabric over
   grid-iron.
3. **M11 plan → `docs/believable_city.md`** (+ M11 pending section,
   README/CHANGELOG links). Committed `af363e3`.
4. **M11 implementation → `0.11.0`** (`05c3f51`): `src/lot.rs` new
   (DistrictFrame, block lattice, 1-D lot strips); framed street hierarchy
   with per-zone arterials; `IS_ARTERIAL/IS_PLAZA/IS_SIDEWALK`; per-lot
   buildings (CBD boost, corner bonus, clumped vacancy); district v2 (CBD
   anchor, adjacency buffer); 4 m scale re-tune; `walk` viz mode;
   `examples/walkability.rs` gate.
5. **Variation complaint → M12 → `0.12.0`** (`3723a2d`, "Full urbanism"
   package): street dropout (~8%, arterials exempt) with 60% greenways
   (`IS_GREENWAY`, bit 5); two global diagonal boulevards (X pair);
   2-D lot packs (≤9); special blocks (Plaza/Market/TowerPark); 7 angles +
   second warp octave.
6. **Determinism question** — answered with evidence (15 focused tests;
   pure `(coords, seed, domain)` functions; same-platform byte equality;
   pre-existing cross-platform libm caveat for transcendentals).
7. **Seam tear (interactive screenshot)** — district borders dead-ended both
   grids. Fix (folded into `3723a2d`): `seam_distance`/`is_seam_road`
   (exact bisector distance from top-2 scan) → arterial parkway both grids
   tee into; threaded through `street_info`, sidewalk ring, tests; verified
   visually in viz.
8. **Interior audit (plan mode)** → M13–M15 roadmap questions answered:
   structure first; MINOR bumps ok (`Cell` frozen); furniture + metadata
   wanted. Roadmap committed `f181126` (plan doc only).
9. **M13 implementation → `0.13.0`** (`6c6a6ad`): lot-true `InteriorContext`
   (pack-rect footprints, corner, frontage depth, `BuildingRole`,
   runner-up zone); floor roles Ground/Typical×N/Top with typical-once-clone;
   stacked shaft (`LAYOUT_CORE`) + `wandering_core`/`vary_typical` opt-ins;
   ground-only entrance + core lobby doors + post-room lobby halo (the halo
   ordering fixed a real narrow-lot starvation bug the truthful footprints
   exposed). New `WorldEngine::voronoi()` accessor.

## Current state

* `main` at `6c6a6ad`; `Cargo.toml` `0.13.0`; M13 ✅ in `Urbix_Project.md` §7
  and README; M14/M15 ⬜ pending with specs in `docs/interiors.md` Roadmap.
* Suite green at commit: 132 lib + integration + 32 doctests;
  `cargo clippy` clean (only pre-existing third-party `block` notice);
  `cargo fmt --check` clean; bench compiles; `walkability` gate passes;
  `cli_demo`/`viz --inspect` smoke-tested.

## Conventions that bit (remember next session)

* Toolchain: `. "$HOME/.cargo/env"` first in every fresh shell.
* Verify: `cargo build --all-targets`, `cargo test`, `cargo clippy
  --all-targets`, `cargo fmt --check` (fmt *before* final test to catch
  drift), plus `walkability`/`viz` smoke for spatial changes.
* Invariants: `Cell` 40 B / header 32 B asserts; `ZoneParams` stayed 16 B
  (padding absorbed `arterial_every`); `CellFlags` bits are additive;
  `InteriorContext`/FFI may grow (MINOR), `Cell` may not.
* cbindgen: new hash domains go in the `exclude` list; `build.rs` carries
  the `ZoneParams` fallback def + `URBIX_FLAG_*` shims; `include/urbix.h`
  regenerates on build (check the diff). `3d-explorer-sdk/.../urbix.h` is a
  stale vendored copy — untouched so far, do not "fix" without asking.
* `InteriorContext::new` / `WorldConfig::interior_context` /
  `chunk::interior_context_for` now take lot-detail args; the latter also
  takes `&VoronoiDiagram`. Examples thread engine/voronoi through.
* Commit style: short imperative subjects (`git log`); local commits only,
  user pushes (passphrase SSH key).
* Test-writing lessons: rotation/split-axis assumptions must be
  seed-agnostic (scan for pairs instead of hardcoding cells); float asserts
  use range-`contains`; `CellFlags::contains` direction is
  `cell.flags.contains(FLAG)`.

## Open threads / next up

* **M14 next** (spec ready in `docs/interiors.md` Roadmap): Blueprint v2
  (min_count, adjacency tags, policies), unit subdivision, wet-stack
  snapping, mixed-use ground. `secondary_zone` is stored but unconsumed —
  M14's entry point.
* Offered but unconfirmed: perpendicular snapping for seam-stub
  T-junctions in the explorer.
* FFI path still regenerates interiors per request instead of using
  `InteriorCache` (slated for M15).
* Pre-existing debt noticed, not touched: `README` version line vs
  `Cargo.toml`; `docs/interiors.md` old line-number references;
  `walkability` "tall cells" is a height proxy, not true landmarks.
