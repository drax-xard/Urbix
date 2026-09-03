# Urbix Demos — Proposals

Three demo tracks that showcase the engine’s capabilities on top of the `0.7.1`
surface (`WorldEngine` `src/engine.rs:52`, wire format `src/data.rs:114`, FFI
`include/urbix.h:1`, CLI `src/main.rs:1`, viz `examples/viz.rs:1`). Each reuses
the same deterministic pipeline (`hash(x,y,seed,domain)` `src/hash.rs:85`,
Shepard Voronoi `src/region.rs:1`, `ChunkCache` `src/cache.rs:40`).

---

## A — Interactive 2D Streaming Explorer (Recommended, implement first)

**Goal:** Prove Urbix is a realtime *infinite streamer*, not a batch generator.

**Stack:** Rust `egui`/`eframe` (single `examples/interactive.rs`, dev-dep, no
`lib` bloat). No new generation logic.

**UX:**
* WASD / drag pans `WorldEngine::set_center` + `generate_chunk`; wheel zooms
  pixels-per-cell. Extent slider (e.g. 2..8) controls `(2*extent+1)²` chunks on
  screen.
* Toggle `hybrid` (zone hue brightened by height, `IS_STREET` black,
  `IS_PARK` green) vs `affinity` (dominant zone flat) — same palette as `viz`.
* `interior_id != 0` dot overlay (proves `src/interior.rs:1` hook).
* HUD: `seed`, `center (cx,cy)`, `chunk_size`, `generated_count`
  `src/engine.rs:203`, `cache_len`/`memory_bytes` `src/cache.rs:183`, frame ms.
* Buttons: `Save PNG` (reuse `viz` writer, PPM+PNG), `Dump bin/json` (reuse
  `src/main.rs:179` `chunk_to_json` / `ChunkBuffer::as_bytes`).

**Data flow:** `egui` input → `WorldEngine` (center + `draw_distance=8`) →
`evict_distant_chunks` → `Cell` grid → `egui::Painter` rects (40 B `Cell`
`src/data.rs:114`). Panning back shows `generated_count` unchanged → cache hit
path `src/engine.rs:122`.

**Why A:** 100% Rust, reuses `viz` hues, stresses LRU + determinism over `i64`
coords (`src/hash.rs:62`), visualizes blend (`docs/world_generation.md:1`) with
~400 LOC, 1 dev-dep, no header change. C FFI is proven separately by
`tests/c_link_run.rs:36`.

**Deliverables:** `examples/interactive.rs`, `README.md` run line, GIF in `docs/`.
Run: `cargo run --example interactive --release`.

**Risks:** `eframe` is heavy to compile; mitigated by dev-dep and `cargo run`
caching. Panning large `extent` (e.g. 8 → 289 chunks) stresses frame — capped
by slider.

---

## B — Web WASM Viewer

**Goal:** Prove language-agnostic claim (`Urbix_Project.md:498` §8.7) in a browser.

**Stack:** `wasm-bindgen` + `wasm-pack`, `WorldEngine` compiled to WASM, JS canvas.
Same Voronoi/chunk pipeline, but exposed via `#[wasm_bindgen]` wrappers around
`WorldEngine::generate_chunk` → `Uint8Array` (wire bytes) or `JSON`.

**UX:** HTML page with canvas, seed input, pan/zoom via mouse, zone overlay
toggle. No install, shareable URL.

**Why B:** Zero-install, viral, proves `hash` determinism across platforms
(SplitMix64 `src/hash.rs:26` stable). Good second demo after A.

**Risks:** `wasm-pack` toolchain, `cbindgen` header not used (WASM has own ABI),
larger bundle. Deferred until A lands.

**Deliverables:** `examples/wasm/` (`lib.rs` wasm glue, `www/index.html`), `README`
`wasm-pack build` line.

---

## C — Godot GDExtension (C FFI)

**Goal:** Prove `staticlib`/`cdylib` (`Cargo.toml:12`) + `include/urbix.h:1`
consumable by a real game engine.

**Stack:** Godot 4 `GDExtension`, import `liburbix.a` via `gdext`/`godot-rust`,
render city as `TileMap` (height → tile height, `palette_id` → tileset).

**UX:** Godot scene where player walks infinitely; engine streams chunks via
`urbix_generate_chunk`/`urbix_chunk_free` `src/ffi.rs:1` behind GDScript.

**Why C:** Strongest proof of “language-agnostic modular engine” objective
(`README.md:12`), opens `§8.7` multi-language bindings.

**Risks:** Heaviest (Godot install, GDExtension boilerplate, `TileMap` art).
Best as third demo after A+B.

---

## Recommendation

Build **A** now; **B** as `examples/wasm` follow-up; **C** as Godot showcase.
All three share the same `WorldEngine` determinism and bounded-memory invariants
(`src/engine.rs:292` 1000-step walk test).
