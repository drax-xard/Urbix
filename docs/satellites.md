# Satellite Builds — Budget & Plan (WASM + Streaming Server)

Status: ⬜ **PENDING** (plan only, no code). For future reference — build only
when a consumer needs it, not speculatively.

Context: the Rust core stays canonical (`src/`, ~10.4k lines, 152 lib tests +
integration + doctests, C ABI, 3-target release tarballs). Both satellites
consume the core as-is; neither forks it. If the core changes underneath
(M16/M17 `WorldConfig` growth), satellites re-test but do not redesign —
their contracts are versioned against the crate version.

Person-day basis: 1 person-day ≈ 6 focused hours (solo-dev pace, this repo's
history: M11–M15 each landed in days, SDK refresh in ~1 week). Estimates
include tests + docs but exclude user-side deploy/ops (pushing, hosting
bills, app-store review).

---

## Satellite 1 — WASM component build ("import urbix anywhere")

### Objective

One `.wasm` artifact exposing chunk + interior + rooms generation to JS and
Python with no C toolchain: `import { generateChunk } from 'urbix'`,
`import urbix` in Python via wasmtime. Proves language-agnosticism better
than any header and unblocks the web viewer from native builds.

### Scope

IN:

- `wasm32-wasip2` component build of the existing crate via `cargo-component`
  with a WIT interface (`generate-chunk`, `generate-interior`,
  `generate-rooms`, `get-zone`, `default-config`).
- JS bindings via `jco` (ESM + types), Python bindings via `wasmtime` sample.
- Byte-equality harness: same `(seed, cx, cy)` → identical bytes WASM vs
  native (modulo documented `f64` formatting at the JSON boundary; raw bytes
  must match exactly).
- Viewer integration spike: `3d-explorer-sdk/www` loads the component for one
  panel (e.g. interior inspector) behind a flag, native server remains default.
- Docs: `docs/wasm.md` (build, versioning, size budget) + README consumer row.

OUT (explicitly): no Gandalf-grade bundler pipeline, no npm publish, no
Python wheel on PyPI, no GPU inside WASM, no threading/SharedArrayBuffer
(work stays single-threaded; worker-pool parallelizes across instances).

### Work breakdown

| # | Task | Size | Notes |
|---|---|---|---|
| W1 | WIT interface + `cargo-component` scaffolding, `wasm32-wasip2` target in CI matrix | 1 d | New `wit/urbix.wit`, `Cargo.toml` crate-type addition; no engine changes. Risk: `cargo-component` version pinning — pin it like `cbindgen 0.29`. |
| W2 | Capability audit (WASI surface: no fs/net/rng in hot path; route file-config loading around the boundary) | 0.5 d | Expect: config arrives as struct/JSON, never a file read inside WASM. |
| W3 | JS bindings (`jco` transpile, ESM + `.d.ts`), minimal web demo page | 1 d | Demo reuses `viz` colour logic in TS; keep it ugly — it's a harness. |
| W4 | Python bindings sample (`wasmtime`), parity test vs native | 0.5 d | `tests/wasm_parity.rs` or a Python script in CI; assert byte equality on 3 seeds × 4 chunks + 2 interiors. |
| W5 | Byte-equality + perf harness (cold-start ms, ms/chunk, `.wasm` byte size) | 0.5–1 d | Budgets: cold start < 300 ms desktop, chunk ≤ 3× native time (WASM overhead), artifact < 2 MB gz. Document, don't gate releases on perf yet. |
| W6 | Viewer integration spike (flag-gated panel in `3d-explorer-sdk/www`) | 1 d | Flag defaults OFF; native path untouched; user eyeballs screenshots (no browser in agent env — same discipline as facade-windows work). |
| W7 | Docs + release plumbing (attach `.wasm` + JS tarball to GitHub Release, versioned with crate) | 0.5 d | Extend `release.yml`; no new hosting. |

**Total: 5–6 person-days.** Calendar: ~1.5 weeks solo with review gaps.

### Acceptance criteria

- `cargo component build --release` green; artifact attached to a test release.
- Parity harness green on 3 seeds (byte-identical chunks/interiors/rooms).
- Web demo renders one chunk + one interior from the component with no native code.
- Size/perf numbers recorded in `docs/wasm.md` (no gate, just truth).
- Main suite untouched and green (`cargo build --all-targets`, `cargo test`, clippy, fmt).

### Risks

- WIT/ABI churn across `WorldConfig` growth (M16/M17) → mitigate: WIT carries a `version()` + config schema hash;JS checks and warns on mismatch.
- `jco`/wasmtime version drift → pin in CI, update quarterly, never floating.
- Performance disappointment (WASM 2–3× slower per chunk) → acceptable: WASM serves tools/viewer panels, never the 60 fps hot path.

---

## Satellite 2 — Streaming server wrapper ("Mapbox for procedural cities")

### Objective

A stateless HTTP server answering `GET /chunks?seed=&cx=&cy=&r=` (grid),
`GET /interior?seed=&wx=&wz=`, `GET /rooms?...`, `GET /config`, backed 1:1 by
the Rust engine with its LRU cache. Unblocks Unity/Unreal/Web/Python clients
that can't or won't link native code, and any multi-client demo.

### Scope

IN:

- Stateless endpoints (every request carries `seed`; server holds no world).
- Batched grid endpoint (`r=` radius, capped like CLI `radius ≤ 64`) reusing
  the existing `3d-explorer-sdk/server/serve.c` JSON shapes where sensible so
  the current viewer works unmodified.
- ETag/`If-None-Match` via chunk content hash (deterministic ⇒ cacheable by
  CDNs for free).
- Dockerfile + `docker compose` local run; health endpoint; request logging.
- Load test: N concurrent clients × M chunks, p95 latency recorded.

OUT: no auth/billing, no persistence (V2 chunk store is separate), no
write/mutation endpoints (§8.6 overlay stays a later epic), no SLA.

### Build option (recommendation: A first)

**Option A — Rust-native (`axum`), recommended.** The server is a second
binary in this repo (`src/bin/urbixd.rs` or `server/` crate) depending on the
`urbix` lib directly. Zero FFI, zero cgo, same types, same tests. This is a
weekend-to-week project, not a rewrite.

**Option B — Go sidecar via cgo.** Only if the deploy environment is
Go-standardized (existing infra, team skill). Same endpoints, ~40% more work
(cgo boundary, type mirroring, two toolchains in CI). Do B only on top of a
proven A (port the HTTP layer, keep the engine).

### Work breakdown (Option A)

| # | Task | Size | Notes |
|---|---|---|---|
| S1 | `urbixd` skeleton (`axum`, routes, config flags, JSON shapes aligned with `serve.c`) | 1 d | Reuse CLI flag conventions (`--port`, `--seed-default`); stateless. |
| S2 | Engine pool (N `WorldEngine` handles behind a semaphore; generation is pure-per-chunk so parallelism is safe with one handle per worker) | 0.5–1 d | Document the existing "handle not thread-safe, pool of handles is" rule from `docs/api.md`. |
| S3 | Batched grid + ETag caching + error contract (bad seed/coords → 400 + JSON, never panic) | 1 d | Radius cap + chunk-count cap per request (DoS discipline). |
| S4 | Interior + rooms endpoints (mirror FFI payloads as base64 or flat JSON arrays; document choice) | 1 d | Prefer flat JSON arrays for debuggability first, base64 blobs if payloads prove heavy — measure. |
| S5 | Dockerfile + compose + CI smoke (`test.sh`-style assertions against live server) | 0.5–1 d | Extend existing `server/test.sh` pattern (currently 22 assertions over `serve.c`). |
| S6 | Load test + numbers (concurrent clients, p95/chunk, RSS ceiling demonstrating bounded memory under walk-sim) | 1 d | Gate: 1000-chunk walk-sim stays under RSS budget; p95 recorded, not gated. |
| S7 | Docs (`docs/server.md`: endpoints, caching, caps, deploy recipe) | 0.5 d | |

**Total (A): 5.5–7.5 person-days.** Calendar: ~2 weeks solo.
**Total (B, Go port after A): +3–4 days** (HTTP port + cgo + CI matrix).

### Acceptance criteria

- All endpoints serve byte-consistent data with the FFI (spot-check harness:
  HTTP chunk bytes == `urbix_generate_chunk` bytes for sampled coords).
- Existing viewer works against `urbixd` with a base-URL switch (no viewer logic changes).
- `test.sh`-equivalent green in CI against the live binary.
- Load numbers recorded (p95, RSS under walk-sim); caps documented.
- Main suite green; no engine changes beyond what endpoints need (none expected).

### Risks

- DoS via huge `r=` grids → mitigate: hard caps (radius, cells/request, concurrent generations per IP), documented 413s.
- Cache poisoning across seeds (LRU keyed wrong) → mitigate: cache key includes `seed + WorldConfig hash`, tested.
- Scope creep into persistence/multiplayer → hard OUT; V2 chunk store and §8.6 overlay are separate epics with separate budgets.

---

## Combined budget & sequencing

| Satellite | Cost | Calendar (solo) | Depends on |
|---|---|---|---|
| 1 — WASM component | 5–6 pd | ~1.5 wks | Nothing (buildable today) |
| 2 — Server, Option A (axum) | 5.5–7.5 pd | ~2 wks | Nothing (buildable today) |
| 2 — Server, Option B (Go, after A) | +3–4 pd | +1 wk | A proven first |
| **Both (A path)** | **10.5–13.5 pd** | **~3–4 wks** | — |

Recommended order: **WASM first** (smaller, proves the versioning story M16/M17 will stress, immediately useful for the web viewer), **server second** (heavier, needs the ETag/caps design). Do not start either mid-milestone — build on a green `main`, and re-run parity after M16/M17 land.

### Ongoing costs (honest footnote)

- CI minutes: +1 matrix leg each (wasm target, server smoke). Trivial.
- Maintenance: version the artifacts with the crate; bump together or not at all.
- Hosting: $0 until you deploy the server publicly (then: one small VPS + CDN caching does a lot, since deterministic content is CDN-perfect). No lock-in: both satellites are stateless and deletable.

## Decision gates (when to actually spend this)

- Build WASM when: a JS or Python consumer exists (viewer panel, notebook analysis, LLM tooling) — not before.
- Build the server when: two or more remote clients need the same city, or native linking blocks a partner (Unity/Unreal team, hosted demo).
- Build neither when: the current FFI + file outputs serve all consumers — which today, they do.

## References

- Core: `Urbix_Project.md` §2 (modules/FFI), `docs/api.md`, `include/urbix.h`.
- Pending core work that satellites must track: `docs/grown_streets.md` (M16), `docs/grammar_massing.md` (M17).
- Existing server precedent: `3d-explorer-sdk/server/` (`serve.c`, `test.sh`, `www/`).
- Origin: `docs/thought_experiment.md` §§1D/1F/2.4.
