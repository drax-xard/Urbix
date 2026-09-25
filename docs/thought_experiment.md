# Urbix Thought Experiment — Alternative Futures

**Status:** brainstorm / non-normative. This document captures a free-form
thought experiment about rebuilding Urbix from different angles while keeping
the core goals in `Urbix_Project.md` §1: convincing varied skyline, infinite
deterministic generation, high-performance realtime, bounded memory,
extensible/modular, clean commented code, truly agnostic (generation only).

**Source of truth reminder:** `Urbix_Project.md` §7 + `README.md` status table
remain canonical. Nothing here changes current behavior unless promoted to a
real milestone.

**Date:** 2026-09-25. Context: Urbix at 0.18.0, Milestones 1–15 done
(exterior lots, streets, districts, finished interiors with programs,
furniture, queryable rooms).

---

## 0. Current DNA (what we keep)

- Deterministic from seed: everything derives from `hash(x, y, seed, domain)`.
  No global RNG, no cross-chunk write dependencies.
- Fuzzy Voronoi districts: 24–48 seed-derived sites mapped to 5 zone types,
  queried continuously (Shepard inverse-distance) for zone affinity.
- Chunked infinite world: default 32×32 cells, 1 cell = 4 m, `Cell` 40 B /
  header 32 B, flat `repr(C)` wire buffers.
- FFI-first / language-agnostic: `include/urbix.h` auto-generated via cbindgen,
  `staticlib` + `cdylib`, ownership transfer (`urbix_chunk_free`,
  `urbix_interior_free`).
- Bounded memory: LRU chunk cache + distance-based eviction, interior cache
  capacity-based.
- Tunable without recompile: `WorldConfig` via TOML/JSON + CLI overrides,
  per-zone `ZoneParams` + `Blueprint` tables.
- Believable-city layer (M11/M12): district frames (rotation/warp),
  street hierarchy (locals, arterials, diagonals, seam parkways, dropout,
  greenways), lots (2-D packs), special blocks, plazas/landmarks.
- Interiors (M13–M15): lot-true `InteriorContext`, floor roles
  (Ground/Typical/Top), stacked cores, unit subdivision, wet stacks,
  mixed-use ground, furniture layer, `UrbixRoom` records, window derivation.

---

## 1. Language Swaps — What If Rust Was Never Picked?

### 1A. Zig — the "C done right" Urbix
- 1:1 transplant. `comptime` replaces cbindgen hacks, `packed struct`
  replaces `_Static_assert`s, `export fn` replaces `#[no_mangle]`.
- Example: `comptime { assert(@sizeOf(Cell) == 40); }`.
- Wins: single binary, zero hidden allocator, one-command cross-compile to
  `x86_64-windows + aarch64-macos + wasm32`, artists `@import("urbix.zon")`.
- Best fit if staying FFI-first but wanting to kill `build.rs` /
  `cbindgen.toml` exclude-list gotchas.
- Tradeoffs: lose Cargo ecosystem, manual memory discipline, smaller hiring pool.

### 1B. Odin + Sokol — the demo-maker's Urbix
- Odin `soa` (struct-of-arrays) + context allocators make ChunkBuffer → mesh
  trivial. Pair with `sokol_gfx.h`: `viz` / `interactive` become a 60 fps
  executable in <500 lines, no `eframe`.
- Ideal if showcase matters more than library purity.

### 1C. C3 / Jai-philosophy — the "no hidden control flow" Urbix
- Contracts, compile-time reflection, real modules without borrow-checker
  friction. True generic fixed arrays that still lower to C ABI — kills the
  `Blueprint[8]` fixed-array + `room_count` dance.

### 1D. Go + gRPC — the server Urbix
- Forget `liburbix.a`. Run `urbixd` as streaming tile server:
  `StreamChunks`, `GetInterior`, `GetRooms`.
- Determinism = free horizontal scaling (any replica answers any cx,cy,seed).
- Add S2/H3 geospatial indexing instead of square chunks → Mapbox for
  procedural cities. Bounded memory becomes per-connection; LRU becomes Redis.
- Boring but instantly usable from Unity/Unreal/Web with no FFI toolchain.

### 1E. Elixir / Erlang (BEAM) — the infinite-city farm
- Each chunk = actor, each district = supervisor. Millions of concurrent
  chunk actors, pre-generate around 1000 players like an MMO.
- Determinism gives idempotent retries. Insane fault tolerance; terrible
  per-cell math — NIF the hot loop in Rust/C.

### 1F. Taichi / Mojo / Halide — the array-language Urbix
- Whole chunk loop is embarrassingly parallel stencil math.
- Taichi `@ti.kernel` auto-parallelized on GPU; Mojo gives Python-importable
  `import urbix` with SIMD, no GIL.
- Measure in MCells/sec on Metal/CUDA; kills criterion benches.

### 1G. Other honorable mentions
- **D + BetterC:** C ABI without runtime, strong `static assert` + `mixin`
  metaprogramming for blueprints.
- **Nim:** Python-like syntax, compiles to C, easy header emission.
- **Swift / Kotlin Native:** if the consumer is mobile-first (AR city walk).
- **APL / J / K:** Voronoi + Shepard in one-liners; unreadable but brutally
  fast to prototype math.

---

## 2. Paradigm Flips — Stop Thinking "Chunk Generator"

### 2.1 GPU-native Urbix — the city is a shader
- Voronoi sites → texture, Shepard blend → fragment/compute shader.
- `urbix_generate_chunk` becomes `dispatch_workgroups(cx,cy)`.
- Streets/lots/buildings all WGSL; CPU reads back height+flags for physics,
  GPU keeps mesh resident. Interiors in second compute pass.
- Result: infinite city ~0 CPU, LOD morphing free.
- Catch: float non-associativity breaks determinism across vendors → fix with
  `u32` fixed-point hash in shader.

### 2.2 Database Urbix — `SELECT * FROM city`
- Ship as DuckDB / SQLite extension:
  `SELECT height, palette FROM urbix_chunks(445566,0,0) WHERE flags & 1 = 0;`
- City becomes queryable. `walkability` becomes SQL. AI agents, data
  scientists, level designers all speak SQL.
- Add zstd on-disk chunk cache + mmap → bounded RAM + unbounded disk, still
  deterministic.

### 2.3 USD / glTF / 3D-Tiles streaming Urbix
- Emit USD prims or Cesium 3D Tiles (`b3dm`/`i3dm`) instead of custom buffer.
- Instant compat with Omniverse, Cesium, Blender, Unreal.
- FFI becomes `urbix_export_usd(cx,cy)`. Stop maintaining `basic_usage.c`,
  maintain a schema.

### 2.4 WASM Component Model Urbix
- Compile core to `wasm32-wasip2`, expose via WIT:
  `generate-chunk`, `generate-interior`, `generate-rooms`.
- Python/JS/Go/Ruby/C# import same `.wasm` sandboxed, no
  `cc -lurbix -ldl -lm -pthread` dance. Hot-swap zone logic by composing
  components.
- True successor to "FFI-first": language-agnostic without ABI fragility.

### 2.5 Node-graph / DSL Urbix (Houdini for cities)
- Replace TOML with visual graph:
  `Voronoi → Warp → BlockSplit → LotPack → Extrude → Facade`.
- Serialize graph to JSON, execute deterministically.
- Artists drag wires instead of editing 8-slot
  `[[interior_blueprints.rooms]]` arrays. Underneath still `hash()`.

### 2.6 Agent-simulation Urbix (form follows process) — L1 PROMOTED to M16
- Simulate land value + traffic + desire paths deterministically.
- Arterials emerge where agents walk, not where `arterial_every=K` says.
  Commercial clusters at high betweenness; industrial repelled from
  residential (today's hardcoded adjacency buffer becomes emergent).
- Same seed → same economy → same city, but believable because it was lived in.
- **Buildable L1 spec: `docs/grown_streets.md`** (site-graph sim → desire
  paths → additive flow arterials; pending Milestone 16). L0 (value-driven
  re-tag) and L2 (footfall lots) stay deferred.

### 2.7 WFC + L-system hybrid
- Keep Voronoi for districts; street topology via Wave Function Collapse
  learned from OSM slices (Manhattan/Barcelona/Kyoto); building massing via
  split grammar (`mass → setback → facade → windows`).
- Swap example inputs to get Haussmann vs. Shinjuku, not `block_size` tuning.

### 2.8 4D Urbix — time as input
- `hash(x,y,t,seed)` where `t` = decade. Same coords in 1920 = brick low-rise,
  2025 = towers, 2070 = arcology. Interiors age (retail → loft).
- Scrubbable growth, gameplay ("rebuild after fire"), diffable urbanism.

### 2.9 LLM-paintable Urbix + MCP server
- Expose as Model Context Protocol tools:
  `generate_district(center, radius, prompt="cyberpunk market, neon, dense")`.
- LLM → `WorldConfig` patch + Blueprint weights → deterministic regen.
  Prompt becomes version-controlled.

### 2.10 Merkle-city — git for procedural worlds
- `chunk_hash = blake3(cells)`; district = hash of children; whole city =
  Merkle tree like Git/IPFS.
- Edits (deferred §8.6 mutable overlay) become commits on procedural base.
  Multiplayer sync via hash exchange; base never re-sent.
- Verifiable generation: prove "tower correct for seed 445566" without
  downloading city.

---

## 3. Better Ways To Hit The Actual Goals

| Goal | Today | Radical upgrade |
|---|---|---|
| Varied skyline | 5 zones, Shepard | Real GIS fusion: blend OSM vectors + procedural fill. Imported Manhattan grid warped by seed downtown, pure procedural outskirts. |
| Deterministic | SplitMix64 | Content-addressed + fixed-point: `i64` coords, `u16` fixed weights, Blake3. Bit-identical CPU/GPU/WASM. |
| Bounded memory | LRU in RAM | Virtual-texture streaming: quadtree LOD (block → district impostor), mmap + zstd, prefetch by velocity. MegaTexture for cities. |
| Agnostic | C ABI | Arrow / FlatBuffers zero-copy (`RecordBatch`, no copy across FFI) or WIT components. |
| Extensible | TOML tunables | Rhai / Lua / Wren scripting: `on_lot(lot) -> height`, hot-reloaded, sandboxed, seeded RNG only. Modders without recompiling. |
| Believable | Lots + arterials | Constraint solver: streets must connect, rooms need daylight, plumbing must stack. Fail loudly instead of slivers. |
| Infinite | Square chunks i32 | Hex chunks + H3 indexing: uniform adjacency, no diagonal special-casing, natural boulevards. Breaks seeds — perfect MINOR excuse. |

Additional wild seeds worth keeping:
- City as function (Shadertoy-like SDF: `city_sdf(x,z,seed) -> height`).
- City as event log (event-sourced mutations + CRDT overlay for multiplayer edits).
- City as audio (sonify density/height for accessibility + generative music).
- City as printer (export lots to CAD/G-code for physical models).
- Night / weather / traffic as pure data layers (§8.5 done right: `urbix_env(x,z,t)`).

---

## 4. Top-3 Prototypes Worth Building

1. **WASM Component + JS viewer (1 weekend).**
   `cargo build --target wasm32-wasip2`, wrap with `jco`, drop into
   `3d-explorer-sdk/www`. Kills `cc` toolchain pain, proves agnosticism
   better than any header.
2. **GPU chunk kernel (1 week).**
   Port `street_info + assign_building` to WGSL, diff pixels vs CPU.
   Unlocks 100× headroom for terrain/water (§8.4).
3. **SQL Urbix (2 days).**
   SQLite virtual table over `WorldEngine`. Walkability metrics become one
   query. Designers will love it.

Next step if promoted: flesh any prototype into M-style spec with file map,
tests, exit criteria (like `docs/believable_city.md`).

---

## 5. Cross-links

- Design: `Urbix_Project.md` §1–§8.
- Pipeline: `docs/world_generation.md`.
- FFI: `docs/api.md`, `include/urbix.h`.
- Interiors: `docs/interiors.md`.
- Believable city: `docs/believable_city.md`.
- History: `CHANGELOG.md`.
