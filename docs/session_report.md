# Session report — city believability + interiors M11 → M15, then SDK + realism to 0.18.0, then M16 grown streets (0.19.0), then M17 grammar massing (0.20.0), then explorer fixes + skyline tuning (0.20.1), then Explorer v2 standalone app

> Working handoff note, not project documentation. Written 2026-09-16,
> last updated after building Urbix Explorer v2 (the Tauri first-person app),
> uncommitted at session end (2026-10-19).

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

## Current state (2026-09-25, post-M16)

* `main` at `33427c6` (+ `900c1a6` audit fixes); `Cargo.toml` `0.19.0`;
  M16 ✅ DONE (flow arterials, `docs/grown_streets.md`); M17 ⬜ pending
  (grammar massing, `docs/grammar_massing.md`); thought-experiment
  (`docs/thought_experiment.md`) and satellite budget (`docs/satellites.md`)
  docs landed as planning references.
* Suite green: 166 lib + integration + 37 doctests; `cargo clippy` clean
  (only pre-existing third-party `block` notice); `cargo fmt --check` clean;
  `cargo doc` back to the 5 pre-existing warnings (1 new redundant-link
  warning caught and fixed in audit); bench compiles with ~0% single-chunk
  delta vs pre-M16; `walkability` and `interiors_gate` gates pass;
  `server/test.sh` all-pass against the vendored 0.18.0 pair.
* SDK still vendored as `urbix-0.18.0-macos-aarch64` (+sha256): header/libs
  are a self-consistent 0.18.0 pair (server builds + tests green), but they
  lag the 0.19.0 engine — re-vendor (header, macOS libs, tarball, doc
  version bumps) rides with the next SDK release, not done yet.
* M16 empirical guarantees (audit-proven, not just argued): `count = 0`
  output is byte-identical to pre-M16 code over 100 A/B chunks (3 seeds +
  negative coords); bench A/B shows no measurable perf delta.

## Session 3 — M16 grown streets + audit (0.18.0 → 0.19.0)

All requested, built, verified, committed locally; user pushes.

23. **M16 implementation → `0.19.0`** (`33427c6`, spec `docs/grown_streets.md`
    from the §2.6 L1 thought experiment):
    `hash.rs` `FLOW_POP/FLOW_JOBS`; `config.rs` `flow_path_count` (8, `0` =
    legacy) + `flow_half_width` (1.0) with serde defaults + `is_valid` +
    strip-and-reparse compat tests; `region.rs` closed-form site-graph
    economy + CBD-pinned desire-path ranking (`total_cmp` + index tie-break)
    + `flow_arterial_at` + 8 unit tests; `chunk.rs` additive flag wiring
    (before sidewalk ring, 15% flow∩grid plazas, flow-aware `abuts_street`)
    + 3 chunk tests + recomputed street-flag test; `street.rs` header note
    (flow deliberately not in `street_info` — frame-independent);
    `cbindgen.toml` excludes; `include/urbix.h` regen (+11 lines, domains
    excluded); example configs + `walkability` ranked-path dump;
    `docs/world_generation.md` pipeline; `Urbix_Project.md` M16 ✅ + module
    map; `README.md` status/version/customization rows; `CHANGELOG.md`
    `0.19.0`. As-built deviations from spec: closed-form single pass (no
    iteration loop), `f32` half-width, `abuts_street` 8th arg with clippy
    allow, always-on path dump instead of a flag.
24. **M16 audit (user: "make sure nothing broke")** (`900c1a6`): worktree A/B
    byte-identity proof (above); edge review (empty/tiny diagrams, NaN
    vectors, sort totality, cast ranges) → one real find: degenerate
    `span = 0` via direct API poisoned keys with NaN → `knee2` floor +
    finiteness test; wiring re-read (plaza single-roll, greenway precedence,
    early-out soundness); consistency sweep (`api.md` needs nothing per
    M13–M15 precedent, `examples.md` blurb, spec "3 passes"/"≤12" leftovers
    corrected); SDK `build.sh` + `test.sh` green as-is.
25. **Doc refresh (this session):** M17/satellites docs re-tensed for landed
    M16, module-map + README customization rows, `CHANGELOG.md` Unreleased
    Fixed entry for the NaN guard, this report regenerated.

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
* M16 lessons: deterministic ranking = `total_cmp` + full index tie-break
  (comparator never returns Equal for distinct items — sort-algorithm
  independent); frame-independent queries (`flow_arterial_at`) live outside
  `street_info` and are OR'd in `chunk.rs`; legacy escape hatches
  (`count = 0`) deserve an *empirical* worktree A/B (100 chunks, neg coords),
  not just a logic argument — it took 10 minutes and settled the question;
  `f32` for small-scale widths, `f64` for world coords; `cargo doc` warning
  count is compared against base (5 pre-existing) on every milestone.

## Open threads / next up

* **M17 grammar massing is next** (`docs/grammar_massing.md` ⬜, targets
  `0.20.0`): `MassingParams` table on `WorldConfig`, `massing_mode = 0`
  legacy gate, no `Cell`/`ZoneParams` growth. Spec already accounts for
  landed M16.
* **SDK re-vendor to 0.19.0** before any SDK work: `sdk/include/urbix.h`,
  `sdk/lib/liburbix.{a,dylib}`, new `urbix-0.19.0-macos-aarch64.tar.gz` +
  `.sha256`, `3d-explorer-sdk/README.md` + `docs/api.md` version bumps
  (0.18.0 → 0.19.0), then `build.sh` + `test.sh`. Linux/Windows tarballs
  still ship from CI `release.yml`.
* Satellites (`docs/satellites.md`): build WASM when a JS/Python consumer
  exists, server when remote clients need the city — decision gates in doc.
* Deferred still: L0 value re-tag + L2 footfall (need M16 avenues locked —
  they are now), G1 WFC fabrics + G2 grammar↔interior bridge (need M17),
  terrain/water (§8.4), road-graph (§8.3), time/weather (§8.5), mutable
  overlays (§8.6).
* Offered but unconfirmed: perpendicular snapping for seam-stub
  T-junctions in the explorer.
* If towers still read slender anywhere: `downtown.slenderness_max = 4`
  in an override file (no recompile), or lower `height_max` further.
  Leftover realism levers (viewer): per-instance facade window grids are
  in; ground-floor retail fronts, chunk-border mass seams, and far-distance
  window fade are not.
* Pre-existing debt noticed, not touched: `docs/interiors.md` old
  line-number references; `walkability` "tall cells" is a relative
  (>1.3× band max) proxy, not true landmarks — its count moves when bands
  move even as the absolute skyline drops. (Fixed 2026-09-25: main `README`
  version lines now say 0.19.0; M16 status row added.)

## Session 4 — M17 grammar massing build (0.19.0 → 0.20.0, DONE)

Built 2026-10-16 as `872fc94` (+ `2e09026` audit fix); `Cargo.toml`
`0.20.0`; M17 ✅ DONE (`docs/grammar_massing.md` + §9 as-built notes).
Built in order 17.1 (`massing.rs` types + pure fn) → 17.2 (`building.rs`
wiring + legacy gate) → 17.3 (`chunk.rs` threading + chunk tests) → 17.4
(config/FFI/examples/docs/release); `Cell`/`ZoneParams` frozen, no
`region.rs`/`street.rs`/interior/flag changes. Suite green: 191 lib +
integration + 41 doctests; clippy/fmt clean; doc warnings at base 5.
Gates at seed 69420/extent 8: `walkability` (20.1/8.8/24.4, wall 16.7) +
`interiors_gate` green; single-chunk bench 657 µs (~+4% vs 0.19.0, inside
the 1.1× budget).

Finds en route, all fixed with regression tests:

* **Cell padding leak** (`src/data.rs`): `set_cell` wrote whole structs,
  leaking 4 uninit implicit-padding bytes into wire output across processes
  (caught because example-file vs CLI-default outputs differed by exactly
  those bytes; all real fields were equal). Now field-at-offset writes.
* **M11 packer aliasing** (`lot.rs`): `lot_slot` partition vs `lot_rect`
  spans disagree by 1 cell at rare boundaries (B=10/n=3 proven). Offsets
  clamp into the rect; `lot_rect` untouched for byte-compat.
* **Market landmark leak** (`src/chunk.rs`, audit fix after the M17
  commit): the first cut folded the landmark boost pre-step for market lots
  in mode 1 (bounded by the 10 m shed cap, but against the spec's "market
  sheds stay low"). Fixed by gating on the ordinary/tower-park exclusion,
  then extracting the shared `lot_boosts` helper with a luck-free unit test
  plus the full-pipeline market recomputation sweep. Lesson: `min()` caps
  mask boost bugs — a discriminating gate (`raw < cap`) matters more than
  fixture count.
* Also: `LAYOUT_PICK`–`LAYOUT_FURNITURE` leaked into `urbix.h` since M9
  (only CORE/WET excluded) — fixed with the `MASS_*` excludes. Boost
  placement made mode-aware (`steps_live`) so flat == legacy bit-exactly.
  `cli_demo` gained a per-cell massing line (no lot-stats section existed).

## Session 5 — visualizers catch up to the framework (2026-10-16)

`viz` + `interactive` + `walkability` predated M13–M17 in places. Updated,
no library changes:

* `viz --mode massing`: podium amber / tower-core hybrid-bright / crowned
  gold (unstepped rules fall back to hybrid); paved keep hierarchy colours.
  Verified by pixel census at 69420/8 (10k podium, 7.8k crown).
* Flow avenues draw teal apart from lattice arterials (`viz` hybrid/walk,
  `interactive` all ground modes). Verified off-center at the path-2
  midpoint (984 teal px) — the locked 69420/8 window has no flow avenues,
  which is why the center renders show none.
* `viz`/`interactive` colour through `config.blended_zone_params` + config
  hues (were using the legacy blend + compiled-in hues; pixel-identical on
  defaults, correct on custom files).
* `viz --inspect` and the interactive selection panel gained the M17
  massing readout (same block as `cli_demo`, kept in sync by hand per the
  M11 duplication precedent); interactive ASCII maps show furnished rooms
  uppercase (M15); mode toggle covers hybrid/affinity/walk/massing.
* `walkability` prints massing role shares (podium 29.4 / tower 47.8 /
  crowned 22.8 / flat 0.0 at 69420/8); gate unchanged.
* `interactive` compiles (eframe) but was not run — no display in this
  environment; needs a human run-through.

## Next session — open threads (as of 2026-10-16)

* `main` at `592371d`; `Cargo.toml` `0.20.0`; M17 ✅ DONE (audited);
  visualizers support the current framework. Suite green: 191 lib +
  integration + 41 doctests; clippy/fmt clean; doc warnings at base 5.
  All work committed locally; user pushes (passphrase SSH key).
* Needs a human: run `interactive` once (new walk/massing modes, flow
  tint, selection massing line, furnished glyphs); eyeball
  `/tmp/v_hybrid|affinity|walk|massing.ppm`-equivalents at seed 69420/8
  plus a flow-avenue area (e.g. `--center-cx 24 --center-cy -138`, path-2
  midpoint) and the `/tmp/smoke` walk output.
* Finetuning entry points (user explicitly wants to finetune later):
  per-zone `massing` rows via `--config` (no recompile), `walkability`'s
  new massing-share line as the metric, `viz --mode massing` as the eye.
* Satellites WASM/server (decision gates in `docs/satellites.md`); L0 value
  re-tag + L2 footfall; G1 WFC fabrics +
  G2 grammar↔interior bridge (needs M17 — now landed, specs not started);
  terrain/water (§8.4), road-graph (§8.3), time/weather (§8.5), mutable
  overlays (§8.6); perpendicular snapping for seam-stub T-junctions
  (offered, unconfirmed); viewer realism leftovers (retail fronts,
  chunk-border mass seams, far-distance window fade).
* Conventions that bit this session (remember next time): example-file
  TOML takes one key per line (`;` only appears inside comments); `min()`
  caps mask boost bugs — write discriminating-gate tests, not just
  fixture-count tests; content renders of `viz` need pixel-census proof
  since images can't be eyeballed in this environment (flow tint at the
  locked center window is correctly zero — avenues don't cross it);
  `cargo test` in debug is fast enough for 200-seed chunk sweeps.

## Session 6 — SDK re-vendor to 0.20.0 (2026-10-19)

Closed the deferred SDK thread. No engine source changes.

* Forced a release rebuild (`touch src/lib.rs && cargo build --release
  --all-targets`) after noticing `target/release/liburbix.*` predated the M17
  audit commit (`2e09026`) by six seconds — the re-vendored pair now provably
  matches `main`.
* Re-vendored `sdk/include/urbix.h` (M16 `flow_path_count`/`flow_half_width`,
  M17 `MassingParams`/`massing_mode`; stale `LAYOUT_*` defines gone),
  `sdk/lib/liburbix.{a,dylib}`, and rebuilt
  `urbix-0.20.0-macos-aarch64.tar.gz` (+ `.sha256`) with the `release.yml`
  layout; removed the 0.18.0 artifacts.
* `docs/api.md`: 0.20.0 title, `MassingParams` + per-zone defaults table,
  flow/massing config fields, corrected `ZoneParams` initializer (adds
  `slenderness_max`), validation ranges. `README.md`: version/provenance/
  tarball bumps + a paragraph on how flow and massing ride the existing chunk
  payload. `CHANGELOG.md` Unreleased entry.
* `server/serve.c` reports `"sdk":"0.20.0"`; `server/test.sh` asserts it.
  Verification green: `build.sh`, all 20 `test.sh` assertions, and all three
  examples compile + run against the vendored pair.

## Session 7 — Explorer fixes + skyline tuning (0.20.1, `90942f2`)

Follow-ups from running the 3D explorer against a real city; released as
`0.20.1` (engine output defaults change, no wire/FFI surface change). One
commit, `90942f2`, 24 files; suite/gates re-verified before and after.

* **Facade-window fix (`server/www/app.js`)**: the old per-chunk instanced
  window boxes (5 cm off the wall, 6 cm thick) read as floating slabs up
  close. Replaced with a procedural window grid in the building material
  (`onBeforeCompile` on Lambert, shared `customProgramCacheKey`; instance
  scale/origin recovered from `instanceMatrix`, face axis from the
  object-space normal, one column per cell and one row per 4 m storey).
  Deterministic lit scatter is unchanged (plus a small emissive term), and
  `fwidth()` fades the grid below ~3 px per window so the old
  `polygonOffset`/far-streak workaround and the 9k-panel budget are gone.
  Verified against the vendored r160 shader text in Node (anchors, balanced
  braces, varyings) and `node --check`; no browser here, so it still needs a
  visual pass.
* **Merge fix (`server/www/app.js`)**: `mergeMasses` replaced its greedy
  row/column expansion (±12% tolerance, routinely beaten by the engine's
  ±10% per-cell jitter) with a 4-neighbour flood fill (`max(2 m, 25%)` of the
  cluster max, bounding-boxed at the cluster height). Sampled 303k cells:
  masses with a side ≤2 cells 95.5% → 63.3%, needles (aspect >6) 52.3% →
  15.7%, median aspect 6.4 → 3.1. Massing steps stay separate, so a 5×5
  downtown lot now renders as one 12 m podium + one 3×3 core.
* **Why buildings read too tall — measured, then fixed in the engine**: the
  height band draw was uniform, so the median lot sat at the band midpoint
  (downtown ~67 m pre-boost), and the `×1.6` outlier clamp / `cbd_factor`
  peak 1.5 / `slenderness_max` 5 stacked on top. Changes: band draw
  right-skewed (`t²`), clamp `×1.3+1`, `cbd_factor` peak 1.3, default
  `slenderness_max` 4. Result on a 1.17M-cell sample: overall median 34.3 →
  23.8 m, downtown 51.7 → 33.2 m (13 → 9 floors), downtown p90 100 → 80 m,
  >80 m share 28% → 5%, max 177 → 144 m; residential median 21.4 → 16.0 m.
* **Re-vendored SDK to 0.20.1** (header byte-identical; libs + tarball
  rebuilt with the `release.yml` layout, old 0.18.0 tarball/sha removed;
  `serve.c`/`test.sh` version strings, SDK `README.md` + `docs/api.md`
  bumped; `api.md` `slenderness_max` 5 → 4 in all rows). Main `README.md`
  version lines → 0.20.1, and the i64 bullet now says coordinates within
  `i32` *hash and locate* identically rather than promising byte-identical
  output (the tuning changes output by design). Gates green at seed 69420
  extent 8: street 20.1%, arterial 8.8%, sidewalk 24.4%, street-wall 16.7
  cells (0.20.0 baseline); `interiors_gate` OK; `viz` walk smoke renders;
  all 20 `server/test.sh` assertions + all three C examples pass.
* Remaining open: human eyeball of `interactive`, the facade shader, and the
  new merge/height tuning; satellites/WASM choices; L0/L2, G1/G2, terrain,
  road graph, weather, overlays as listed above.

## Current state — 0.20.1 (2026-10-19)

* `main` at `90942f2` (release 0.20.1), local only — **user pushes**.
  `Cargo.toml` `0.20.1`. M17 ✅ DONE; visualizers support the framework;
  SDK vendored as `urbix-0.20.1-macos-aarch64` (self-consistent, matches the
  engine). 24 files changed in the release commit.
* Suite green: 191 lib + integration + 41 doctests; `cargo clippy
  --all-targets` clean; `cargo fmt --check` clean; `cargo build
  --all-targets`; `cargo test --doc`. Spatial gates at seed 69420/extent 8:
  `walkability` (street 20.1 / arterial 8.8 / sidewalk 24.4, street-wall
  16.7 cells) + massing shares (podium 29.4 / tower 47.8 / crowned 22.8) —
  unchanged vs the 0.20.0 baseline; `interiors_gate` OK; `chunk_gen` bench
  compiles. Viewer: `node --check` + the Node shader-injection check pass.
* `0.20.1` is a PATCH with output-default changes only: no `Cell`/
  `ZoneParams`/`WorldConfig`/FFI/flag change, header byte-identical.
* Docs refreshed this session: `AGENTS.md` project state → M17 + the
  vendored-SDK rule (deliberate re-vendor, old "leave it alone" retired);
  `src/massing.rs` / `src/building.rs` comments 1.6× → 1.3×; CHANGELOG
  `[0.20.1]`; `docs/world_generation.md`, `grammar_massing.md`,
  `believable_city.md`, SDK `README.md`/`docs/api.md`; this report.

## Open threads (as of 2026-10-19)

* **Needs a human eyeball** (no browser/GL in this environment): the
  procedural facade shader (flush windows, `fwidth()` LOD fade), the new
  `mergeMasses` flood fill (masses not needle-like), the retuned skyline,
  and a run-through of `interactive` (walk/massing modes, flow tint,
  selection massing line, furnished glyphs). Serve with `./server/serve
  --port 8311 --seed 445566 --web server/www` and hard-refresh.
* **Finetuning entry points** (user wants to finetune later): per-zone
  `massing` rows and `slenderness_max` via `--config` (no recompile),
  `walkability`'s massing-share line as the metric, `viz --mode massing`
  as the eye. Leftover realism levers (viewer): ground-floor retail fronts,
  chunk-border mass seams, far-distance window fade are still not done.
* **Specs not started:** G1 WFC fabrics + G2 grammar↔interior bridge (M17
  is landed, specs aren't); terrain/water (§8.4), road-graph (§8.3),
  time/weather (§8.5), mutable overlays (§8.6); L0 value re-tag + L2
  footfall (M16 avenues are locked).
* **Satellites** (`docs/satellites.md`): build WASM/server when a consumer
  exists — decision gates in the doc. **Perpendicular snapping** for
  seam-stub T-junctions in the explorer: offered, unconfirmed.
* **Pre-existing debt** (untouched): `docs/interiors.md` old line-number
  references; `walkability` "tall cells" is a relative (>1.3× band max)
  proxy whose count moves with the bands even as the absolute skyline drops.

## Session 8 — Urbix Explorer v2 standalone app (2026-10-19, UNCOMMITTED)

Built a standalone first-person 3D explorer as a **new independent crate**
`explorer-v2/` (NOT a workspace member of the root `urbix` package, so the
root build stays light). Plan doc: `docs/explorer_v2.md` (status line already
says "planned → implemented"). Nothing here is committed yet — all files are
untracked/modified per `git status`; **user pushes**.

### What it is

Tauri 2 (Rust shell + system WKWebView) wrapping the evolved Three.js viewer,
with the backend calling the `urbix` crate **directly as a path dependency**
(`../..`). No C server, no HTTP, no vendored SDK. Frontend is vanilla ES
modules + vendored Three.js r160 — **no npm, no build step** (Tauri serves
`../src` via `frontendDist`). Alternatives rejected: keep v1 C-server+browser
(not standalone); Bevy native rewrite (too much renderer rebuild);
macroquad/SDL2+OpenGL (loses the polished shaders).

### Deliverables

* `docs/explorer_v2.md` — approved plan (motivation, framework decision,
  controls, build/run, verification, risks).
* **Backend** `explorer-v2/src-tauri/`:
  - `src/engine.rs` — `EngineState { engine: Option<WorldEngine>, config:
    WorldConfig }` (derives `Default`), `SharedEngine = Mutex<EngineState>`,
    `CommandError`, DTOs (`CellDto`, `ChunkDto`, `FloorDto`, `InteriorDto`,
    `RoomDto`, `ConfigDto`, `SpawnDto`); 7 commands `get_config`,
    `start_world`, `find_zone_start`, `get_chunks`, `get_interior`,
    `get_rooms`, `get_zone`; helpers `is_walkable_ground`, `dominant_zone`,
    `tile_byte`, `ring_cells`, `chunk_to_cells`, `resolve_zone_spawn`,
    `scan_disk_for_zone` (`ScanParams` struct); `DISTRICT_SCAN_RADIUS = 384`.
    7 unit tests pass (incl. `find_zone_start_resolves_every_zone_across_seeds`
    over seeds 445566/1/69420/999).
  - `src/main.rs` — `Mutex<EngineState>` + `generate_handler!` for all 7.
  - `Cargo.toml` (tauri 2.12.4, serde, serde_json, urbix path `../..`),
    `build.rs` (tauri-build), `tauri.conf.json` (`withGlobalTauri: true`,
    window 1280×800, `frontendDist: "../src"`, CSP `default-src 'self';
    script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:;
    connect-src 'self' ipc: http://ipc.localhost`), `capabilities/default.json`,
    `gen_icons.py` + `icons/`.
* **Frontend** `explorer-v2/src/`:
  - `index.html`, `style.css`, `app.js` (module, imports
    `./vendor/three.module.js`); `vendor/three.module.js` only (unused
    OrbitControls removed, importmap removed to keep strict CSP with no inline
    scripts).
  - Menu (seed/zone/coords/START) → `start_world` (seed sent as a **string** so
    full `u64` survives JSON IPC) → `find_zone_start` → spawn at ground level on
    a street in the nearest matching-zone district. FPS: pointer-lock mouselook
    (request from a user gesture), WASD + Shift, cell-based collision, eye
    height 1.7. ENTER center-raycast against building instance meshes → enter
    that building's ground floor via `get_interior`/`get_rooms`; ESC exits to a
    street cell just outside the lot; R/F switch storeys. Chunk streaming
    radius 3 via batched `get_chunks`, LRU-ish eviction on radius exit.
    Facade shader, mass merging, paving, furniture/window/room meshes ported
    from v1.
* **Engine additive changes** (root crate): `CellFlags::bits()` accessor in
  `src/data.rs`; `pub const VERSION: &str = env!("CARGO_PKG_VERSION")` in
  `src/lib.rs`. No wire/FFI/`Cell` changes.
* **Housekeeping**: `.gitignore` now ignores
  `explorer-v2/src-tauri/{target/,Cargo.lock,gen/schemas/}`;
  `CHANGELOG.md` `[Unreleased]` entry.

### Verification (all green)

* Root crate: `cargo build --all-targets`, `cargo test` (191 lib +
  integration + 42 doctests), `cargo clippy --all-targets`, `cargo fmt --check`.
* `explorer-v2/src-tauri`: `cargo build`, `cargo test` (7/7),
  `cargo clippy --all-targets` (clean — fixed `EngineState` derive, the
  8-arg `scan_disk_for_zone` → `ScanParams`, `contains_key`+`insert` →
  `entry().or_insert_with`, `map_or(true, ..)` → `is_none_or`, test
  `-1 * n` → `-(n as i32)`), `cargo fmt --check`, `cargo build --release`
  (binary at `target/release/urbix-explorer-v2`).
* `node --check` on `app.js` (parses as an ES module); vendored
  `three.module.js` confirmed a real ES module (`const t="160",...`).

### Not done / needs a human

* **Never run the GUI** (no display here): the menu → START → walk → ENTER →
  ESC flow, WKWebView pointer-lock, and the ported facade/interior rendering
  all need an eyeball.
* **No `.app` bundle**: `cargo-tauri`/`tauri` CLI not installed and
  `/usr/bin` is not writable for a clean install — `cargo build --release`
  yields a runnable binary (Tauri embeds `../src` at build time). To bundle:
  `cargo install tauri-cli` then `cargo tauri build` from
  `explorer-v2/src-tauri`.
* **Homebrew rust `/opt/homebrew/bin/rustc` is broken** — use rustup; always
  `. "$HOME/.cargo/env"` first (prepend `~/.cargo/bin` or run
  `. "$HOME/.cargo/env"` in every shell). node/npm live at `~/.local/bin`.

## Open threads (as of 2026-10-19, end of Session 8)

* **Everything from Session 8 is uncommitted** — review `git status`, commit
  locally (imperative subjects; user pushes).
* **Eyeball Explorer v2**: run the release binary, verify menu/spawn/ENTER/ESC
  and that the ported rendering matches v1; consider `interactive` + the
  facade shader + retuned skyline (still pending from Session 7).
* **Explorer v2 polish candidates** if wanted after the eyeball: ground-floor
  retail fronts, chunk-border mass seams, far-distance window fade (the same
  viewer realism levers noted in Session 7); optionally a `tauri` bundle.
* **Engine**: `0.20.1` unchanged this session (only additive `CellFlags::bits`
  + `urbix::VERSION`); no re-vendor triggered (no ABI/output change). If any
  future engine change moves output/ABI, re-vendor the SDK per the AGENTS.md
  rule.
* **Still open from Session 7**: SDK vendored 0.20.1 self-consistent;
  satellites/WASM gates (`docs/satellites.md`); L0/L2, G1/G2, terrain (§8.4),
  road graph (§8.3), weather (§8.5), overlays (§8.6); perpendicular snapping
  for seam-stub T-junctions; `docs/interiors.md` stale line refs;
  `walkability` "tall cells" relative proxy.

## Session 9 — commit Explorer v2 / release 0.21.0 (2026-10-21)

* Session 8 tree committed as `b196ba8` ("Release 0.21.0: add standalone
  Explorer v2"), 26 files.
* Engine `Cargo.toml` → `0.21.0` (additive `CellFlags::bits()` +
  `urbix::VERSION`); `CHANGELOG.md` `[0.21.0]` absorbs the Unreleased
  entries; `README.md` version lines bumped. No wire/FFI/output change, so
  the SDK stays vendored at 0.20.1.
* Explorer `Cargo.lock` is now tracked (root ignore anchored to
  `/Cargo.lock`); `docs/explorer_v2.md` corrected to the shipped command
  surface.
* Verified: root build/tests (191 lib + 4 bin + 4 integration + 42 doctests),
  clippy, fmt; explorer build/test (7/7), clippy, fmt, release; `node --check`.
* Still needs the same human eyeball and polish threads listed above:
  menu/spawn/ENTER/ESC flow, facade shader, merged masses, retuned skyline;
  then retail fronts, chunk-border seams, and far-window fade.
