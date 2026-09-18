# Session report — city believability + interiors M11 → M15, then SDK + realism to 0.18.0

> Working handoff note, not project documentation. Written 2026-09-16,
> last updated after the 0.18.0 realism pass (2026-09-18).
> Tree is clean at `4b77ec9`.

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
12. **AGENTS.md touch-up + report committed** (`341f72b`): milestone line to
    M14, schema-evolution Gotcha (serde defaults + strip-and-reparse +
    `is_valid`).
13. **M15 implementation → `0.15.0`** (`5dda131`): furniture sets per kind
    stamped into a parallel `Floor.furn` layer (primary always, secondary
    rolls `furn_density`; tile semantics frozen; payload ×3 grids, prefix-
    compatible); `Floor::window_cells` + renderer recipe (no wire tile);
    `rooms_of_floor` + published `unit_rects_for_floor`/`building_shafts`/
    `split_units`/`PlaceRegion`; additive `urbix_generate_interior_rooms`/
    free (`UrbixRoom` 10 B, header-asserted); engine-side `InteriorCache`
    on both FFI paths (`interior_cache_len`); `examples/interiors_gate.rs`
    (synthetic + 18 sampled lots); ASCII uppercase furnished rooms, report
    stats. Notable fix en route: my own FFI assert compared furn tiles to
    `== 1` (Wall) instead of `== 5` (Room) — caught by the test run.

## Session 2 — 3D-explorer SDK refresh + realism pass (0.15.0 → 0.18.0)

All requested, built, verified, committed locally; user pushes.

14. **SDK refresh plan (plan mode) → `3f71186` (0.15.0 vendored).**
    Audited the 0.10.0→0.15.0 drift (3-grid interior payload, room records,
    M11/M12 street-hierarchy flags, Blueprint v2, grown `WorldConfig`) and
    rebuilt: re-vendored header/libs + release tarball, rewrote
    `3d-explorer-sdk/docs/api.md`, updated `explore_grid/interior.c`, added
    `explore_rooms.c`; `serve.c` gained `/api/rooms`, batched
    `/api/chunks?r=`, live `/api/config`, `--chunk-size/--draw-distance`,
    portable `build.sh`; viewer gained paved-hierarchy overlay, walk mode
    (V), teleport (T), batch streaming + perf HUD, furniture + derived
    window glazing + room/unit stats; `test.sh` 18 assertions.
15. **Double-click entry (`793dcaa`).** Single-click teleports hijacked
    orbit drags starting on a building. Exterior entry is now `dblclick`
    (interior pointer-lock click + G/Enter unchanged); hints updated.
16. **"Too tall and thin" diagnosis (plan mode).** Measured, not guessed:
    heights are metres, lots are 12–20 m wide, Downtown ran 40–200 m × up
    to 1.5 × 1.2 (clamp 321 m) → up to 25:1 vs real 3:1–6:1. User chose
    engine retune + regenerate + chunky mid-rise.
17. **Config-vs-hardcode question** — answered with file/line evidence:
    bands/knobs are file-tunable (`urbix.toml`, `--config`, FFI
    `WorldConfig`); shaping math + compiled defaults are engine. Offered
    demo `--config` for no-rebuild A/B — accepted.
18. **`urbix_default_config()` + demo `--config` → `6752589` (0.16.0).**
    Additive FFI getter (round-trip test) so C starts from defaults and
    patches fields. `serve --config` loads a minimal `KEY = VALUE`
    override file (zone bands/density/blocks, floor height) over engine
    defaults, CLI flags win; `/api/config` reports effective
    `height_min/max`; `server/chunky.overrides` shipped as the experiment
    (Downtown 24–110 m etc.). A/B same seed: max 273.6→152.0 m, mean
    153.6→87.0 m, >100 m buildings halved, density identical.
19. **Promote chunky bands → `14e87cb` (0.17.0).** Downtown 24–110,
    Commercial 10–45, Industrial ≤20, Residential ≤14 in `config.rs` +
    `zones.rs` + both example files. Only the FFI getter test referenced
    absolute bands; everything else is relative. Default seeds regenerate.
20. **Slenderness clamp → `a81c9c8` (0.18.0).** New `ZoneParams.
    slenderness_max` (default 5, serde-defaulted, `is_valid` ≤ 12, snapped
    from dominant zone): height ≤ K × narrowest lot side in metres, `0`
    disables. `assign_building` grew a `lot_footprint` arg (`lot::lot_rect`
    at the `chunk.rs` call site) via shared `slenderness_cap()` helper,
    re-applied after landmark/special boosts so both sites agree.
    `ZoneParams` exactly fills 16 B now (was 15 + pad). Measured: >100 m
    buildings 4963→1844, mean 87→82 m, density untouched.
21. **Viewer massing (`0f00419`) + facade windows (`5cc3681`, `726e0cd`).**
    Screenshot showed per-cell needles: `mergeMasses()` fuses touching
    same-zone/palette cells within 12% height into single boxes at group
    max (greedy rects; logic proven via Python port, 300 fuzz trials —
    no JS runtime on this machine). Then per-cell-per-storey instanced
    facade panels (dark glass + 22% warm lit scatter, 9k/chunk budget with
    row-halving fallback). Also unstacked the status/popup HUD overlap.
22. **Window streak fix (`4b77ec9`).** Far windows smeared: 0.035 offset
    below depth precision under a 0.1–2000 frustum. Fix =
    `polygonOffset(-2)` + 0.05 offset + far plane 2000→1200 (fog opaque
    past 560, nothing lost).

## Current state

* `main` at `4b77ec9`; `Cargo.toml` `0.18.0`; milestones M1–M15 ✅ (no new
  milestones; 0.16.0–0.18.0 are unplanned realism/FFI releases).
* Suite green at commit: 152 lib + integration + 36 doctests;
  `cargo clippy` clean (only pre-existing third-party `block` notice);
  `cargo fmt --check` clean; bench compiles; `walkability` and
  `interiors_gate` gates pass; `test.sh` 22/22; C examples compile;
  viewer changes eyeballed by user from screenshots (no browser here).
* SDK re-vendored as `urbix-0.18.0-macos-aarch64` (+sha256); Linux/Windows
  still ship from CI `release.yml`.

## Conventions that bit (remember next session)

* Toolchain: `. "$HOME/.cargo/env"` first in every fresh shell.
* Verify: `cargo build --all-targets`, `cargo test`, `cargo clippy
  --all-targets`, `cargo fmt --check` (fmt *before* final test to catch
  drift), plus `walkability`/`viz` smoke for spatial changes. `cargo fmt`
  (write) fixes the long-call drift rustfmt flags.
* Invariants: `Cell` 40 B / header 32 B asserts; `ZoneParams` is now
  *exactly* 16 B (4×f32 + 4×u8, no padding left — next u8 needs layout
  work); `CellFlags` bits are additive; `InteriorContext`/FFI may grow
  (MINOR), `Cell` may not.
* cbindgen: new hash domains go in the `exclude` list; `build.rs` carries
  the `ZoneParams` fallback def (update its field list too) +
  `URBIX_FLAG_*` shims; `include/urbix.h` regenerates on build (check the
  diff). The `3d-explorer-sdk` header/libs/tarball are re-vendored
  deliberately on every engine change (exception to the old "don't touch"
  rule — that note is retired).
* `assign_building` now takes `lot_footprint: (u8, u8)`; post-boost
  clamps in `chunk.rs` must reuse `slenderness_cap()` (single rule, two
  sites). `blended_zone_params`/`zone_params` snap policy fields
  (`block_size`, `arterial_every`, `slenderness_max`) — never blend them.
* Commit style: short imperative subjects (`git log`); local commits only,
  user pushes (passphrase SSH key).
* Test-writing lessons: rotation/split-axis assumptions must be
  seed-agnostic (scan for pairs instead of hardcoding cells); float asserts
  use range-`contains`; `CellFlags::contains` direction is
  `cell.flags.contains(FLAG)`; when a rolled outcome misses on one seed,
  scan seeds for the fixture rather than weakening the assert — and prefer
  making the generator rule strict (skip instead of sprawl) so tests can
  stay exact.
* Viewer has no JS runtime on this machine: prove new `app.js` logic by
  porting the algorithm to Python and fuzzing it; get the user to eyeball
  screenshots. `test.sh` covers server JSON only.
* `sed -i '' 's/.../.../g'` version bumps also hit historical "since X"
  references — re-check and revert those lines.

## Open threads / next up

* **No pending milestones** — §7 is all green through M15. Natural next
  epics (none specced): enter/exit teleport API, terrain/water (§8.4),
  road-graph navigation (§8.3), time/weather data (§8.5), dynamic overlays
  (§8.6), language bindings / WASM (§8.7).
* Offered but unconfirmed: perpendicular snapping for seam-stub
  T-junctions in the explorer.
* If towers still read slender anywhere: `downtown.slenderness_max = 4`
  in an override file (no recompile), or lower `height_max` further.
  Leftover realism levers (viewer): per-instance facade window grids are
  in; ground-floor retail fronts, chunk-border mass seams, and far-distance
  window fade are not.
* Pre-existing debt noticed, not touched: main `README` version lines
  (still say 0.9.0) vs `Cargo.toml`; `docs/interiors.md` old
  line-number references; `walkability` "tall cells" is a relative
  (>1.3× band max) proxy, not true landmarks — its count moves when bands
  move even as the absolute skyline drops.
