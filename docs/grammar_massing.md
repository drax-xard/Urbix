# Grammar Massing — Split-Grammar Buildings (Milestone 17, G0)

Status: ⬜ **PENDING** (plan only, no code). Parent: `Urbix_Project.md` §7 M17,
`docs/thought_experiment.md` §2.7 (G0 row).

This document is the buildable specification for the G0 slice of the
WFC + L-system hybrid: **split-grammar building massing with learned streets
deferred**. No WFC, no OSM data, no adjacency tables in this milestone —
only the grammar half, and only massing (heights), not facade dressing.
Streets (`street.rs`), lots (`lot.rs`), and interiors (M13–M15) are untouched;
`building.rs` stops extruding flat lots and starts splitting them.

Locked decisions:

1. **Massing only, heights only.** The wire carries one `height` per cell —
   grammar steps compress into per-cell height deltas (podium ring vs tower
   core, roof ridge, sawtooth bays). No `Cell` change (40 B asserts stay),
   no new flags, no palette change (one palette per lot, as today).
2. **Flat lots are depth-0 grammar.** Every rule has a zero that reproduces
   legacy output: `massing_mode = 0` (or all-zero params) is byte-identical
   to pre-17 `assign_building` for the same seed+config (escape hatch + test
   anchor).
3. **`ZoneParams` frozen (exactly 16 B).** All new knobs live in a
   `MassingParams` table on `WorldConfig` (MINOR bump, serde defaults, header
   regen) — same pattern as M9 `interior_blueprints`.
4. **Bounded depth, pure functions.** Split trees expand to a fixed max depth
   (rings computed arithmetically, never recursed open-ended). Every draw is
   `(lot_id, cell_offset, seed, domain)`-pure — chunk edges agree for free.
5. **Scale canon unchanged:** 1 cell = 4 m. Podium floors convert via the
   existing `interior_floor_height` (default 4.0 m ≈ 1 cell per storey —
   convenient: 1 podium floor ≈ full-cell height step).

## 1. Goals / non-goals

Goals:

* Kill the extruded-rectangle skyline: podiums, setback towers, pitched
  houses, sawtooth sheds — five zones read as five architectures from the air.
* Tune without recompile: per-zone massing tables in TOML/JSON like
  blueprints.
* Compose with everything pending: M16 flow avenues get lined with stepped
  masses; slenderness clamp keeps authority (steps never outgrow the lot).

Non-goals for M17:

* No WFC streets, no OSM/fabric tables (G1 — deferred, separate spec).
* No facade dressing (window grids, cornices, shopfronts) — height steps only.
* No grammar↔interior bridge (G2 — setbacks stay exterior; `InteriorContext`
  keeps the full pack rect, documented as a known approximation).
* No new wire semantics: a renderer that extrudes `height` per cell gets
  stepped masses with zero client changes.

## 2. Design

```
lot (id, rect w×d, corner, block_clump, cbd_boost — as today)
  │
  ▼
base height H = existing pipeline (zone band → clump → cbd → corner → 1.6× clamp)
  │                                  (unchanged code path, then steps below)
  ▼
massing_for(lot_id, rect, cell_offset, H, massing_params, seed):
  ring = min(ox, oz, w-1-ox, d-1-oz)          // inset ring index, 0 = edge
  base = podium_cap(ring)                      // edge rings capped to podium top
  peak = roof_shape(ox, oz, rect, roof_mode)   // pitched / sawtooth multiplier
  crown = crown_roll(lot_id) ? 1.12 : 1.0      // inner cells only
  jitter = ±10% per cell (existing HEIGHT domain, applied last as today)
  height = min(base * peak * crown * jitter, slenderness_cap(...))
```

### 2.1 `MassingParams` — one row per zone

```rust
#[repr(C)] // FFI-crossable, serde-tunable, like Blueprint
pub struct MassingParams {
    pub podium_floors: u8,  // full-footprint base height in floors (0 = no podium split)
    pub tower_inset: u8,    // rings from the lot edge capped to the podium (0 = flat)
    pub setback_every: u8,  // upper-mass step-back rhythm, reserved for G2 depth (0 = off in G0)
    pub roof_mode: u8,      // 0 flat, 1 pitched (ridge along long axis), 2 sawtooth (bays along x)
    pub crown_share: f32,   // share of lots earning a +12% crown cap on inner cells (0..=1)
}
```

Defaults (code, mirrored into `urbix.toml/json.example`):

| Zone | podium / inset / every / roof / crown | Reads as |
|---|---|---|
| Downtown | 3 / 1 / 0 / flat / 0.30 | tower-on-podium, occasional crown |
| Commercial | 2 / 1 / 0 / flat / 0.15 | shopfront base + short tower |
| Residential | 0 / 0 / 0 / pitched / 0.0 | house with ridge roof |
| Industrial | 0 / 0 / 0 / sawtooth / 0.0 | shed bays with skylight rhythm |
| Park | 0 / 0 / 0 / flat / 0.0 | flat shed (unchanged) |

`setback_every` is accepted and validated in G0 but only *wires* in G2
(multi-tier towers need per-floor massing the wire can't carry yet) — document
as reserved, test that nonzero parses and validates but does not alter output
(keeps the schema stable across G0→G2 without a second bump).

### 2.2 Per-cell rules (all pure, all bounded)

* **Podium split** (`tower_inset > 0`): cells with `ring < tower_inset` cap at
  `podium_top = podium_floors × interior_floor_height` (floored at 1 cell of
  height so a 0-podium config never zeroes); inner cells keep full `H`.
  Single-storey lots (`H <= podium_top`) are unaffected — no-op by
  construction, covered by test.
* **Pitched roof** (`roof_mode = 1`): ridge along the lot's long axis;
  `peak = 1.15` on the ridge line falling linearly to `0.90` at the eaves
  (pure integer arithmetic on offsets — no trig, no libm widening).
* **Sawtooth** (`roof_mode = 2`): bay bands along x with period 3 cells,
  alternating `1.12 / 0.94` from a `lot_id`-keyed phase (deterministic per
  lot, seamless across chunks since phase derives from `lot_id`, not chunk).
* **Crown** (`crown_share > 0`): lot-level roll on `domain::MASS_CROWN`
  (`hash < crown_share`); winning lots scale inner cells (`ring >=
  tower_inset`) by `1.12`. Edge cells never crown (caps stay street-walled).
* **Ordering:** podium → roof → crown → jitter → `min(1.6× band clamp,
  slenderness_cap)` — caps apply last through the shared
  `slenderness_cap()` helper at both call sites (existing two-site
  discipline: `building.rs` + `chunk.rs` post-boost re-clamp).
* **Empty lots** unchanged: fate roll first, `(0.0, 0)` short-circuits before
  any massing math.

### 2.3 What does NOT change

* `assign_building` signature grows by `(rect, offset)`-style lot geometry it
  can already see at the call site (`lot::lot_rect` is computed in `chunk.rs`
  today for the slenderness clamp) — prefer threading the already-computed
  rect + per-cell offset over recomputing.
* Palette: one per lot, same domain, same counts.
* Jitter: same `HEIGHT` domain, same ±10%, applied after steps so life
  survives on every mass.
* Landmark/special-block boosts (`×1.5`, TowerPark `×1.35`) multiply `H`
  before steps — a landmark tower keeps its podium + crown, just taller.
* Interiors: `interior_context_for` keeps the full pack rect (setbacks are
  sub-cell-mass illusions on the heightfield, not footprint cuts). G2 will
  reconcile; G0 documents the approximation.

## 3. Config + FFI + compat

`WorldConfig` gains (appended — payload growth appends, never interleaves):

| Field | Type | Default | `is_valid` | Meaning |
|---|---|---|---|---|
| `massing_mode` | `u8` | `1` | `<= 1` | `0` = legacy flat extrude (byte-identical), `1` = grammar |
| `massing` | `[MassingParams; 5]` | table above | per-row checks | per-zone split rules |

* `#[serde(default)]` shims so pre-17 files parse (prove with the
  strip-and-reparse round-trip test per `config.rs` convention).
* `is_valid`: `massing_mode <= 1`; per row `crown_share` in `0..=1`,
  `roof_mode <= 2`, `podium_floors <= 32`, `tower_inset <= 8`,
  `setback_every <= 32` (reserved, validated, unwired).
* `ZoneParams` untouched (full at 16 B). `Cell` untouched (40 B asserts stay).
  `CellFlags` untouched.
* `WorldConfig` size grows → MINOR bump to `0.20.0` (on top of M16's
  `0.19.0`; if M16 is unbuilt when this lands, the bump covers both and the
  spec notes the stack). `include/urbix.h` regenerated.
* `MassingParams` is a new exported type: add to `cbindgen.toml` `include`
  list, and mirror the `ZoneParams` precedent in `build.rs` (manual fallback
  def + asserts inside the guard) if cbindgen omits it. New hash domains
  `MASS_PODIUM = 62`, `MASS_CROWN = 63`, `MASS_ROOF = 64` → add all three to
  the `exclude` list (gotcha: every new domain leaks into the header
  otherwise).
* Text-file sketch:

```toml
massing_mode = 1   # 0 disables grammar (legacy extrude exactly)
[[massing]]        # index 0 = Downtown (ZoneType order)
podium_floors = 3; tower_inset = 1; setback_every = 0; roof_mode = 0; crown_share = 0.3
# ... 4 more [[massing]] to fill the fixed array (same fixed-array TOML
# discipline as interior_blueprints: all 5 slots present when specified)
```

## 4. File deliverables

| File | Change |
|---|---|
| `src/hash.rs:35` | New domains `MASS_PODIUM: 62`, `MASS_CROWN: 63`, `MASS_ROOF: 64` |
| `src/massing.rs` (new) | `MassingParams` (`repr(C)`, serde, defaults table), `massing_for()` pure step fn, `is_flat()` legacy check; re-export in `lib.rs` |
| `src/building.rs` | `assign_building` threads rect+offset through `massing_for` (§2.2 ordering); caps via shared `slenderness_cap()`; `cell_jitter=None` tests assert exact step heights |
| `src/chunk.rs` | Pass already-computed `lot_rect` + cell offset (no recompute); post-boost re-clamp unchanged; interiors untouched (documented approximation) |
| `src/config.rs` | `massing_mode` + `massing: [MassingParams; 5]` + serde defaults + `is_valid` + file round-trip; example-file comments |
| `src/engine.rs` | Nothing (config rebuild path already covers it) — cover with a test |
| `src/ffi.rs`, `build.rs`, `cbindgen.toml`, `include/urbix.h` | `WorldConfig` + `MassingParams` export; regen header; exclude new domains |
| `urbix.toml.example`, `urbix.json.example` | New `massing_*` lines with 4 m-scale comments |
| `examples/viz.rs`, `cli_demo.rs`, `examples.md` | No new modes (heights already visualized); `cli_demo` reports podium/tower/crown shares in its lot stats |
| `docs/world_generation.md`, `docs/api.md` | Pipeline (massing step) + config sections; no wire-format change |
| `docs/thought_experiment.md` | §2.7 points here as the promoted G0 spec (G1/G2 stay deferred) |

## 5. Determinism and cross-chunk rules

* All steps are integer offset arithmetic + seeded hashes — no trig, no float
  sort, no iteration. Same `(lot_id, offset, seed)` → same height on any
  platform (strictly narrower libm surface than Shepard, which already uses
  `powf`-free rational math per M16).
* Phase/rolls key off `lot_id` (stable per block+slot), never chunk — sawtooth
  bays and crowns align across chunk borders by construction.
* `massing_mode = 0` (or zeroed table) is byte-identical to pre-17 output
  (gateway test pins this on a sampled chunk grid).
* Default seeds regenerate with stepped masses (expected MINOR break, same
  precedent as M11/M12/M16 — no legacy fallback beyond mode `0`).

## 6. Tests and exit criteria

Unit:

* Legacy gate: `massing_mode = 0` byte-equality vs pre-17 on sampled chunks.
* Determinism: same lot twice → same steps; podium ring strictly shorter
  than tower core on multi-storey downtown lots; single-storey lots unaffected.
* Crown share: statistical (seed sweep, `contains` range — never hardcode a
  seed per repo convention); edge cells never crowned.
* Pitched: ridge cells taller than eaves on residential lots; symmetric lots
  symmetric within jitter-off tolerance.
* Sawtooth: 3-cell period present on industrial lots; phase stable across
  chunk borders (same-lot pair scan, seed-agnostic).
* Reserved: nonzero `setback_every` parses, validates, and does not alter
  output (locks G0→G2 schema stability).
* Config: old files parse (strip-and-reparse), `is_valid` rejects
  out-of-band rows/modes; layout asserts (`Cell` 40 B, `ZoneParams` 16 B) stay
  green.

Property / gates:

* Slenderness still holds everywhere (steps capped after, not before).
* Street-wall continuity does not regress (podium rings preserve frontage
  height — assert mean uninterrupted frontage vs 0.18.0 baseline).
* `walkability` + `interiors_gate` green (massing must not move paved shares
  or room stats — heights only).
* Perf: `cargo bench --bench chunk_gen` delta within noise (< 1.1× — a few
  integer ops + 1 extra hash on crowned lots only); document result.
* Manual: `viz` overhead — downtown podiums + tower steps + crowns, houses
  ridged, sheds banded; `walk` mode — podiums read as shopfront bases.

Exit criteria: all green + gates pass + `0.20.0` bump + `CHANGELOG.md` +
`Urbix_Project.md` M17 ✅ + README status row.

## 7. Sequencing

Build in order; each step testable alone: 17.1 (`massing.rs` types + defaults
+ pure fn + unit tests) → 17.2 (`building.rs` wiring + legacy gate) →
17.3 (`chunk.rs` threading + caps + chunk tests) → 17.4 (config/FFI/examples/
docs/metrics). G1 (WFC fabrics) and G2 (grammar↔interior bridge) each get
their own spec afterwards — neither starts until massing is locked, since both
consume `MassingParams`, not the reverse.

## 8. Risks

* Podiums read as wedding cakes everywhere (too uniform) → mitigated by
  per-lot crown/podium-floor rolls keyed on `MASS_*` (not fixed per zone) +
  existing ±10% jitter + block clumping; validate via `viz` before locking
  defaults.
* Pitched roofs on narrow lots alias into spikes → mitigated by ridge math in
  integer space + slenderness cap applied after steps; test narrowest lots
  explicitly.
* `WorldConfig` growth stacking with M16 → mitigated by documenting the
  combined bump if M16 is unbuilt; appending (never interleaving) keeps old
  readers' prefix slices valid per repo wire discipline.
* Artists want full grammar tuning now (rule trees, not 5 scalars) →
  explicitly out of scope; the fixed-row table is the same compromise
  `Blueprint` made in M9 and it scaled fine through M14.

## References

* Pipeline: `docs/world_generation.md`, `src/chunk.rs`, `src/building.rs`,
  `src/lot.rs`, `src/config.rs`, `src/hash.rs`.
* Wire/FFI: `docs/api.md`, `src/data.rs`, `src/ffi.rs`, `include/urbix.h`.
* Fabric spec: `docs/believable_city.md` (§2 scale, slenderness).
* Flow avenues (composes): `docs/grown_streets.md` (M16).
* Origin: `docs/thought_experiment.md` §2.7.
