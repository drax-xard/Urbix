# Session report — city believability + interiors M11 → M14

> Working handoff note, not project documentation. Written 2026-09-16,
> last updated after M14. Tree is clean at `dc2e074`.

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
10. **AGENTS.md refresh + this report committed** (`15e392b`): fixed stale
    v0.1.0/placeholder claims and the un-wired-cbindgen note; added smoke
    commands, Gotchas (cbindgen excludes, wire invariants, contains
    direction, seed-agnostic tests, vendored SDK header), docs pointers.
11. **M14 implementation → `0.14.0`** (`dc2e074`): Blueprint v2
    (`min_count`, `TAG_WET/QUIET/PUBLIC/STREET`, `doors`, `unit_max`,
    `wet_shafts`, `ground_zone`, `corridor` — all serde-defaulted, compat
    proven by strip-and-reparse test); guillotine unit subdivision with
    shaft-aware cuts (no unit orphaned from every stack), per-unit anchors,
    minimums-first-then-fill, room-less guarantee (non-wet), unit front
    doors; wet-stack snapping (`LAYOUT_WET`, building-constant columns,
    fill-phase skips so every placed wet room provably stacks);
    `resolve_ground_override` + `generate_layout_with_ground` (retail base
    under 3+ storey homes); multi-door rooms; single-loaded band policy.
    Fixture lesson reused: seed 10 rolls no retail (1-in-9) — mixed-use
    test scans seeds for its fixture instead of hardcoding.

## Current state

* `main` at `dc2e074`; `Cargo.toml` `0.14.0`; M14 ✅ in `Urbix_Project.md` §7
  and README; M15 ⬜ pending with spec in `docs/interiors.md` Roadmap.
* Suite green at commit: 143 lib + integration + 32 doctests;
  `cargo clippy` clean (only pre-existing third-party `block` notice);
  `cargo fmt --check` clean; bench compiles; `walkability` gate passes;
  `cli_demo`/`viz --inspect` smoke-tested; ASCII floor maps eyeballed
  (stacked shaft, entrance, kitchen/bath alignment).

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
  `cell.flags.contains(FLAG)`; when a rolled outcome misses on one seed,
  scan seeds for the fixture rather than weakening the assert — and prefer
  making the generator rule strict (skip instead of sprawl) so tests can
  stay exact.

## Open threads / next up

* **M15 next** (spec ready in `docs/interiors.md` Roadmap): furniture on
  reserved `LAYOUT_FURNITURE`, windows (derive renderer-side first),
  additive `urbix_generate_interior_rooms` + free fn, FFI path through
  `InteriorCache`, extended `interior_report` + interiors gate example.
  Entry points: `PlacedRoom` deliberately keeps rects (unit rects available
  via `split_units`); unit doors are plain `Door` tiles awaiting unit tags
  in room records.
* Offered but unconfirmed: perpendicular snapping for seam-stub
  T-junctions in the explorer.
* Pre-existing debt noticed, not touched: `README` version line vs
  `Cargo.toml`; `docs/interiors.md` old line-number references;
  `walkability` "tall cells" is a height proxy, not true landmarks.
