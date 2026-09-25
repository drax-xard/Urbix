# Grown Streets — Flow Arterials (Milestone 16, L1 Agent-Sim)

Status: ✅ **DONE** (released in 0.19.0). This document was the build
specification; the implementation follows it as built. Notes on as-built
deviations are marked [AS-BUILT] inline.

This document is the buildable specification for the L1 slice of
agent-simulation Urbix: **arterials emerge from simulated flow, not from
`arterial_every = K`**. The per-zone lattice, dropout, greenways, diagonals,
and seam parkways all stay. Flow paths are *additive* world-space avenues on
top of them — the same trick diagonals and seams already use, so
cross-chunk consistency comes for free.

Locked decisions:

1. **Simulate sites, not cells.** The economy runs once on the 16–64-site
   Voronoi graph at `generate_with_config` time. Per-cell cost is one
   point-to-segment pass over a handful of paths — no agents stepped per chunk.
2. **Additive, not replacement.** The `arterial_every` lattice is untouched.
   Flow arterials OR onto `IS_ARTERIAL`. `flow_path_count = 0` reproduces
   legacy output byte-identically (escape hatch + test anchor).
3. **`Cell` frozen, `ZoneParams` frozen.** `ZoneParams` is exactly 16 B
   (4×f32 + 4×u8, zero padding left). All new knobs live on `WorldConfig`
   (MINOR bump, serde defaults, header regen). No new `CellFlags` bits:
   flow avenues reuse `IS_STREET + IS_ARTERIAL`.
4. **Arithmetic-only sim.** No `exp`/`sin` in the economy loop — only `+ - * /`
   and one rational falloff (same family as `cbd_factor`'s Lorentzian) — so
   the existing cross-platform libm caveat does not widen.
5. **Scale canon unchanged:** 1 cell = 4 m (`docs/believable_city.md` §2).

## 1. Goals / non-goals

Goals:

* Avenues that radiate from the economic core and connect subcenters, instead
  of a perfect avenue grid — the city reads *grown* from the air and stays
  permeable on foot.
* Commercial follows movement: flow-adjacent lots earn the existing
  Commercial-adjacent treatments (shopfront depth rule, plaza rate) without
  new flags.
* Same hard invariants: pure `hash(x, y, seed, domain)`, no cross-chunk
  writes, LRU-bounded memory, FFI-first.

Non-goals for M16:

* No live per-chunk agents, no footfall field (that's L2 — deferred).
* No zone re-tagging from the sim (L0 value field stays informational; the
  M11 CBD anchor + adjacency buffer keep authority).
* No road-graph / pathfinding (§8.3 stays deferred; flow paths are geometry,
  not a stored graph).
* No terrain coupling (§8.4 stays deferred).

## 2. Design

```
sites (positions + zones, as today)
  │
  ▼
16.1 economy sim (once, at generate_with_config):
  pop[i], jobs[i] from hash → gravity traffic[i->j] → value[i], flow[i]
  │
  ▼
16.2 desire paths: top-N pairs by traffic → segments in world coords
  stored on VoronoiDiagram as Vec<FlowPath> (immutable, whole-run)
  │
  ▼
16.3 per-cell (chunk.rs, world-space like diagonals/seams):
  flow_arterial_at(x, z) = min over paths of point-to-segment dist <= half_width ?
  → IS_STREET + IS_ARTERIAL (dropout-immune, sidewalk ring respects it)
```

### 16.1 Economy sim (once per diagram)

Inputs: site positions `(x, y)`, zones, `seed`. New hash domains `FLOW_POP`
(60), `FLOW_JOBS` (61) — site-index-keyed draws via `hash_coords(i, 0/1,
seed, domain)`, same pattern as `SITE_X/Y`.

Per site `i`:

* `pop[i] = 0.3 + 0.7 * hash_unit(i, 0, seed, FLOW_POP)`, scaled by zone
  capacity: Residential ×1.3, Downtown ×1.1, Commercial ×1.0,
  Industrial ×0.6, Park ×0.15.
* `jobs[i] = 0.3 + 0.7 * hash_unit(i, 1, seed, FLOW_JOBS)`, scaled:
  Downtown ×1.5, Commercial ×1.3, Industrial ×1.0, Residential ×0.4,
  Park ×0.05.

Pairwise traffic (gravity, rational falloff — no transcendentals):

```
d2 = (xi-xj)² + (yi-yj)²
traffic[i->j] = pop[i] * jobs[j] / (1 + d2 / knee²),  knee = span * 0.25
```

Closed form, evaluated once in site-index order (same snapshot discipline
as the adjacency buffer in `region.rs`):

```
value[i] = jobs[i] + sum_j traffic[j->i] * 0.5 - pollution[i]
pollution[i] = sum over Industrial j of jobs[j] / (1 + d2(i,j) / knee²) * 0.4
flow[i] = sum_j traffic[i->j] + sum_j traffic[j->i]   // through-traffic
```

([AS-BUILT] the plan first sketched this as 3 fixed passes; the formulas do
not recur, so the build evaluates them directly — no iteration loop, one
fewer determinism surface, same outputs.)

Outputs stored per site (two `Vec<f32>` beside `sites`, or a small
`SiteEconomy { value, flow }` array — private, never FFI):

* `value[i]` — informational in M16 (drives path endpoints + future L0 work;
  does NOT re-tag zones, does NOT replace `cbd_factor`).
* `flow[i]` — ranks path endpoints.

Determinism notes: closed-form single evaluation, accumulation in site-index
order, all `f64` arithmetic (same as Shepard today). Document the summation
order in code comments — it is part of the wire-stable output.

### 16.2 Desire paths (once per diagram)

* Candidate pairs: all `i < j` with `traffic[i->j] + traffic[j->i]` ranked
  descending. Skip pairs where both endpoints are Park (no avenue to nowhere).
* Take top `config.flow_path_count` (default 8, `0` disables → legacy).
* Tie-break: lower `(i, j)` first — fully deterministic, no float-sort
  instability (sort by `(traffic_bits_desc, i, j)` on ordered integer keys,
  or stable sort with index tie-break documented).
* Each path stored as:

```rust
pub struct FlowPath {
    pub ax: f64, pub ay: f64,   // endpoint A (site position)
    pub bx: f64, pub by: f64,   // endpoint B
    pub weight: f32,             // normalized traffic share, informational
}
```

* Endpoint snap: paths run site-to-site straight (world coords). They cross
  districts and chunks seamlessly — same construction as `Diagonal`.
* CBD discipline: the CBD-anchor site (nearest origin, forced Downtown) is
  pinned as an endpoint of path #0 (paired with its highest-traffic partner)
  so every city has one legible radial — then remaining paths take the global
  ranking. Document the pin explicitly; it keeps the skyline peak and the
  avenue star aligned.

### 16.3 Per-cell query (hot path)

New `VoronoiDiagram` method (pure, world-space, chunk-consistent):

```rust
pub fn flow_arterial_at(&self, world_x: f64, world_z: f64, half_width: f64) -> bool
```

* Point-to-segment distance to each path (≤ 16 paths, early-out on first hit).
  Squared-distance formulation, no `sqrt` until the final compare — or compare
  in squared space outright.
* `half_width` from config (default 1.0 cells → 2-cell avenues, matching the
  lattice arterial width and diagonal `half_width: 1.0`).
* Returns `true` → caller sets `IS_STREET + IS_ARTERIAL`, height 0, no-build.

Wiring in `chunk.rs` (after the existing `street_info` + diagonal + seam
resolution, before sidewalk ring):

```
flow = voronoi.flow_arterial_at(wx, wz, config.flow_half_width)
if flow { flags |= IS_STREET | IS_ARTERIAL; info.street = true (for sidewalk/plaza logic) }
```

Rules inherited (no special cases):

* Flow arterials are **immune to dropout** (like lattice arterials,
  boulevards, seams) — the city stays permeable at avenue scale.
* The **sidewalk ring** treats flow cells as streets (they already are
  `IS_STREET` by the time the neighbour check runs — order the flag write
  before the ring pass).
* **Plazas:** flow∩lattice-grid crossings earn plazas at the diagonal rate
  (15% via `domain::PLAZA`, same call site as diagonal crossings) — boundary
  boulevards already proved this reads well. No new domain.
* **Greenways/special blocks/landmarks:** unchanged. Flow avenues do not
  create or suppress them.

Perf budget: ≤ 16 segments × ~10 flops per cell on avenue-candidate cells
only — hoist behind the existing street-hit early-out where possible
(lattice/diagonal/seam hits skip the flow pass; flow only runs on cells that
would otherwise be block interior). `cargo bench --bench chunk_gen` must show
< 1.3× vs `main` (tighter than M11's 2× because the sim itself is free).

## 3. Config + FFI + compat

`WorldConfig` gains (appended — payload growth appends, never interleaves):

| Field | Type | Default | `is_valid` | Meaning |
|---|---|---|---|---|
| `flow_path_count` | `u8` | `8` | `<= 16` | desire paths kept; `0` = legacy, byte-identical |
| `flow_half_width` | `f32` | `1.0` | `0.5..=2.0` | avenue half-width in cells |

* `#[serde(default)]` shims so pre-16 files parse and get defaults (prove
  with the strip-and-reparse round-trip test per `config.rs` convention).
* `is_valid` guards above; FFI setters reject invalid (null/invalid → no-op,
  never unwind — existing `ffi.rs` discipline).
* `ZoneParams` untouched (full at 16 B). `Cell` untouched (40 B asserts stay).
  `CellFlags` untouched (reuse `IS_ARTERIAL`).
* `WorldConfig` size grows → MINOR bump to `0.19.0`, `include/urbix.h`
  regenerated, `urbix.toml.example` / `urbix.json.example` gain commented
  `flow_*` lines, `build.rs` fallback untouched (no `ZoneParams` change).
* New hash domains `FLOW_POP = 60`, `FLOW_JOBS = 61` → add to `cbindgen.toml`
  `exclude` list (gotcha: every new domain constant must be excluded or it
  leaks into the header).

Text-file sketch:

```toml
flow_path_count = 8   # 0 disables flow avenues (legacy grid exactly)
flow_half_width = 1.0 # cells; 1.0 = 2-cell avenue like lattice arterials
```

## 4. File deliverables

| File | Change |
|---|---|
| `src/hash.rs:35` | New domains `FLOW_POP: 60`, `FLOW_JOBS: 61` |
| `src/region.rs` | `SiteEconomy { value, flow }` (private); sim in `generate_with_config` (§2, 16.1); `FlowPath` struct + `flow_paths()` accessor; `flow_arterial_at()` query (§2, 16.3); CBD pin documented |
| `src/chunk.rs` | Flow wiring (§2, 16.3): flag write before sidewalk ring; flow∩grid plaza rate; keep `door_side_for` / lot pipeline untouched |
| `src/street.rs` | No signature change (flow resolved in `chunk.rs`, not `street_info`) — document why in module header; dropout immunity falls out of flag ordering |
| `src/config.rs` | `flow_path_count`, `flow_half_width` + serde defaults + `is_valid` + file round-trip; example-file comments |
| `src/engine.rs` | Nothing (Voronoi rebuild on `set_config` already re-runs the sim) — cover with a test |
| `src/ffi.rs`, `build.rs`, `cbindgen.toml`, `include/urbix.h` | `WorldConfig` growth only; regen header; exclude new domains |
| `urbix.toml.example`, `urbix.json.example` | New `flow_*` lines with 4 m-scale comments |
| `examples/viz.rs`, `walkability.rs`, `examples.md` | `flow` overlay colour (arterial tint already covers it; add a `--flow-paths` debug dump printing path endpoints/weights); walkability asserts avenue connectivity, not just share |
| `docs/world_generation.md`, `docs/api.md` | Pipeline + config sections; no wire-format change to document |
| `docs/thought_experiment.md` | §2.6 points here as the promoted L1 spec |

## 5. Determinism and cross-chunk rules

* Sim inputs are site positions + seed-hashed pop/jobs — both fixed at
  `generate_with_config`. Iteration count (3) and accumulation order (index
  order, snapshot discipline) are code constants.
* Paths are world-space segments; per-cell distance uses absolute coords +
  `div_euclid`-free pure arithmetic — two chunks querying the same world
  point compute the same answer (same argument as diagonals/seams).
* `flow_path_count = 0` is byte-identical to pre-16 output for the same
  seed+config (gateway test pins this).
* Old seeds with default config regenerate with the new avenues (expected
  MINOR break, same as M11/M12 — no legacy fallback beyond `count = 0`).

## 6. Tests and exit criteria

Unit:

* Sim determinism: same seed → identical `value/flow/paths` across runs;
  path endpoints reference real site indices; Park–Park pairs never emitted.
* CBD pin: path #0 touches the origin-nearest (Downtown-forced) site.
* `flow_arterial_at`: true on path midpoints, false 5 cells off-path;
  negative world coords agree (sign-stability).
* `count = 0` byte-equality vs legacy lattice on a sampled chunk grid.
* Config: old files parse (strip-and-reparse), `is_valid` rejects
  `count > 16` / `half_width` out of band; `set_config` rebuilds paths
  (engine test).
* Layout asserts: `Cell` 40 B, `ZoneParams` 16 B stay green.

Property / gates:

* Walkability: avenue share rises vs `main` baseline but block perimeters
  stay in `believable_city.md` §2 bands; 4-way vs T-junction ratio does not
  collapse (radials add T's at the lattice — bound the delta); 1000-step walk
  still bounded.
* Street-wall continuity does not regress vs 0.18.0 baseline.
* Perf: `cargo bench --bench chunk_gen` < 1.3× vs `main`; document result.
* Manual: `viz` overhead — one radial star at the CBD + 2–4 cross-links,
  no avenue running Park-to-Park into nowhere; `walk` mode — flow avenues
  read as boulevards with sidewalks, crossings get occasional plazas.

Exit criteria: all green + `walkability` + `interiors_gate` gates pass +
`0.19.0` bump + `CHANGELOG.md` + `Urbix_Project.md` M16 ✅ + README status
row.

## 7. Sequencing

Build in order; each step testable alone: 16.1 (sim + accessors + tests) →
16.2 (paths + ranking + pin + tests) → 16.3 (per-cell wiring + plaza rate +
chunk tests) → 16.4 (config/FFI/examples/docs/metrics). Do not start L2
footfall or L0 zone re-tagging until avenues are locked — they consume
`value/flow`, not the reverse.

## 8. Risks

* Avenue star overwhelms the lattice downtown (too much pavement) → mitigate
  with default 8 paths + `count` knob; validate via walkability paved-share
  before locking the default.
* Radial/grid crossings read as broken, not grand → mitigated by the proven
  diagonal-crossing plaza rate + dropout immunity (same cure as seams).
* Float-sort nondeterminism in ranking → mitigated by integer-keyed ranking
  with index tie-break (§2, 16.2); test with adversarial seeds.
* Default-seed churn (all cities regenerate) → accepted MINOR break with a
  `count = 0` exact-legacy escape hatch, same precedent as M11/M12.

## References

* Pipeline: `docs/world_generation.md`, `src/chunk.rs`, `src/street.rs`,
  `src/region.rs`, `src/config.rs`, `src/hash.rs`.
* Wire/FFI: `docs/api.md`, `src/data.rs`, `src/ffi.rs`, `include/urbix.h`.
* Fabric spec: `docs/believable_city.md` (§2 scale, §12–§13 avenues/seams).
* Origin: `docs/thought_experiment.md` §2.6.

## 9. As-built notes (0.19.0)

* [AS-BUILT] The economy sim is a **closed-form single pass**, not "3 fixed
  passes": every output (traffic, pollution, value, flow) derives directly
  from the hashed draws with no recurrence, so no iteration loop exists.
  Stronger than specced — one fewer determinism surface, same outputs.
* [AS-BUILT] `flow_arterial_at` takes `half_width: f32` (coordinates stay
  `f64` for span precision; width is small-scale cell units).
* [AS-BUILT] `abuts_street` grew an 8th argument (`flow_half_width`) and
  carries `#[allow(clippy::too_many_arguments)]` with justification, the same
  precedent as `building::assign_building` — full pipeline context per
  neighbour reads clearer than a bundle struct here.
* [AS-BUILT] The `--flow-paths` dump landed as **always-on lines** in
  `walkability` (ranked endpoints + weights after the gate) rather than a
  flag — 8 deterministic lines, no parser plumbing, same debuggability.
* [AS-BUILT] Perf: single-chunk bench shows **no measurable delta**
  (~631 µs vs ~630 µs baseline, p = 0.42) — far inside the 1.3× budget. The
  avenue early-out plus the empty-vec fast path make flow ~free.
* [AS-BUILT] Walkability at seed 445566 / extent 8: street 20.2%, arterial
  8.8% (up from lattice-only — the expected avenue-share rise), sidewalk
  24.5%, street-wall mean 17.0 cells (68 m, no regression), 530 junctions.
  `walkability` + `interiors_gate` gates green.
