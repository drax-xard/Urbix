# Project Design — Urbix

A platform-agnostic and language-agnostic modular engine for generating an explorable, infinite procedural city that can be used/called from other engines either via CLI or API.

---

## 1. Objectives

### Design objectives
1. **A convincing, varied skyline.** Distinct urban regions — skyscraper
   business districts, tranquil residential areas, busy commercial zones,
   dirty industrial zones, and green parks — each with its own density,
   building height distribution, and color scheme.
2. **Infinite world, deterministic generation.** The city is generated in
   chunks on demand, so it is effectively unbounded while
   remaining reproducible from a seed.


### Performance objectives
3. **High-performance realtime.** 
4. **Bounded memory.** Chunks are cached with a distance-based eviction limit
   so infinite generation never grows unbounded in RAM.

### Engineering objectives
5. **Extensible and modular.** Core generation engine is transparently extensible with modules to alter or expand its capabilities.
6. **Clean and commented code :** Uses best coding practices to generate sensible code with high quality comments, verbose logging and useful test sets 
7. **Truly agnostic :** The engine only objective is to generate the city; other aspects like rendering, UI, gameplay and everything else is abstracted to whatever software is making use of this engine.

---

## 2. Architecture Overview

(This section should detail how the engine will be structured and what each file should do or provide.

---

## 3. World Generation — Voronoi Regions with Fuzzy Borders

Generation is layered so that *regions* (big, smooth) and *chunks* (local,
deterministic) compose into one seamless infinite city.

### 4.1 Region layer (districts)
- A **Voronoi diagram** of a fixed set of sites (e.g., 24–48) spread across a
  large coordinate span, computed once from the seed. Each site is tagged with
  a zone type from the following set:
  - **Downtown / Business** — dense, tall skyscrapers.
  - **Residential** — tranquil, low-rise, tree-lined.
  - **Commercial** — busy mid-rise with bright/neon palettes.
  - **Industrial** — grimy, wide low warehouses, open lots.
  - **Park / Green** — low or zero buildings, foliage, open ground.
- **Fuzzy (soft) borders:** each query blends contributions from the *nearest
  two* sites using a weighted distance ratio (e.g., smoothstep/interpolated by
  the second-nearest distance). The result is a per-point *zone affinity
  vector* across a small palette of parameters (density, height range, color
  palette, street/block style). This yields **gradual, seamless transitions**
  between adjacent districts instead of hard edges, and naturally keeps
  neighboring chunks consistent with one another.

### 4.2 Chunk layer
- World is divided into fixed **chunks** (e.g., 32×32 cells), addressed by
  integer `(cx, cy)` chunk coordinates.
- Each chunk's contents are generated **deterministically** from
  `hash(cx, cy, seed, zone_param_blob)`. No persistent global RNG and no
  cross-chunk write dependency — any chunk can be (re)built on demand and
  adjacent chunks agree at their shared edges because they query the same
  continuous Voronoi/zone field.
- **Street layout & blocks:** a per-zone street grid with block sizes tuned by
  region (tight small blocks downtown, roomy blocks residential, large wide
  blocks industrial). Streets have `height = 0`; interior of each block is
  filled with a building footprint (or open/park cells).
- **Per-building variation:** individual building height and facade
  palette-id are derived by hashing cell coordinates, clamped into the zone's
  height range, so every built cell looks a little different.

### 4.3 Cache & eviction
- Chunks are materialized lazily around the player and kept in an LRU cache
  keyed by `ChunkId`.
- Chunks far outside the draw distance are **evicted** (and dropped) so memory
  stays bounded even as the player explores infinitely.
- The *region* Voronoi map is tiny and immutable; it lives for the whole run.

### 4.4 Interior hooks (future)
Every built lot (each building/home/factory footprint) is a candidate for a
future interactable interior. To make that evolution cheap and seamless, the
world model **already computes an `InteriorId` for every built cell today**,
even though interiors are not yet rendered:

- `InteriorId = hash(cell_coords, seed, zone)` is deterministic and stable, so
  a given doorway always leads to the *same* room across visits and across
  runs with the same seed.
- The `interior` module defines the **hook surface** (function signatures /
  trait) a future renderer and teleport routine will implement:
  - `InteriorState` — what an interior run needs (room layout, size, fog,
    palette, exits).
  - `enter(lot, player)` / `exit(lot, player)` — teleport into/out of a
    deterministic interior.
  - `generate_interior(id) -> InteriorState` — stub returning a placeholder
    so the *interface* is wired and callable end-to-end.
- Until implemented, `interior` exposes a **null/generic `InteriorState`** and
  the enter action is a no-op (or clearly logged), so the data plumbing and
  the player-interface affordance exist without the feature being shown.
- Design note: an interior is a *separate mini-world* (its own small grid,
  not part of the infinite chunk city), keyed by `InteriorId`, so it can be
  generated and cached independently of the outdoor chunks.

---

## 5. Versioning & changelog
The project tracks a **semantic versioning** (SemVer) scheme and maintains a
`CHANGELOG.md`:

- **Version format** `MAJOR.MINOR.PATCH`:
  - `MAJOR` — breaking changes to the build.
  - `MINOR` — backward-compatible features (new zone, new mode, new flag).
  - `PATCH` — bug fixes and polish with no new surface.
- `CHANGELOG.md` follows the **Keep a Changelog** convention:
  - `[Unreleased]` section at top collects pending changes.
  - Categorized entries: `Added`, `Changed`, `Fixed`, `Removed`,
    `Performance`, `Documentation`.
  - Each milestone and each significant feature gets a tagged release.

---

## 6. Documentation & Code Comments

The codebase carries **quality, informative documentation** at every layer so
it is approachable and self-explanatory.

### 6.1 Doc comments (rustdoc)
- Every **public item** (module, routine, function, and
  significant field) gets a comment explaining *what* it does, *why*
  it exists, and (where non-obvious) *how* to use it.
- Modules open with a comment block describing their role in the overall
  architecture (mirroring the module map in section 2).
- Doc comments include an *example* where it clarifies usage.
- Intended audiences: a new contributor should be able to read the documentation and
  understand the whole pipeline without reading the source.

### 6.2 Inline comments
- Use **explanatory comments for the *why* and the *algorithm*** — especially
  in the deterministic generators, where the math is otherwise opaque.
- **Explain every nontrivial step** of the hardest parts to re-derive.
- Prefer naming + structure over comment walls: a helper with a clear name and
  a one-line doc beats a five-line bullet comment.
- **Do not** restate the obvious (`// increment i`); comments justify
  non-obvious decisions, edge cases, and invariants.
- Mark intentional placeholder code clearly with `TODO`/`FIXME` so the hooks and placeholder sections are
  visible to future work without being mistaken for finished features.

### 6.3 Project-level documentation
- `project-design.md` — this document: objectives, architecture, and the
  development plan (living document, updated as decisions change).
- `README.md` — the top-level entry point: what the project is, a screenshot
  description, quickstart (build + run), CLI flags, and a pointer to
  the design doc and changelog.
- `CHANGELOG.md` — versioned history.
- `docs/` — deeper write-ups (world generation, api structure, etc)
  created with the milestones, referenced from the README.

---

## 7. Development Plan (milestones)

(This section will describe each of the milestones and steps needed to build the engine)

---

## 8. Future Extensions (explicitly deferred)

(This will include ideas for future development beyond the initial objectives delineated in the project design.