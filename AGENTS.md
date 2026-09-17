# AGENTS.md

Guidance for working in this repository.

## Project state

Urbix is a deterministic, infinite procedural city engine (Rust crate).
Generation is implemented through Milestone 15 (exterior lots, streets,
districts, and fully finished interiors with programs, furniture, and
queryable rooms). Source of truth for what exists is the milestone table
in `Urbix_Project.md` §7 and the status table in `README.md` — not this
file's history.

## Toolchain (important gotcha)

Rust is installed via rustup but is **not on the default shell PATH**. In any
new shell or command, source it first:

```sh
. "$HOME/.cargo/env"
```

## Commands

Verification order (run all after a change):

```sh
cargo build --all-targets
cargo test
cargo clippy --all-targets
cargo fmt --check    # fix with: cargo fmt
```

Example/bench scaffolding require a `fn main()` to compile; keeping them
compiling is required since `cargo build --all-targets` and `cargo test` build them.

For spatial/generation changes also smoke-test (release mode; debug is too
slow for image-sized output) and gate on the headless metrics:

```sh
cargo run --release --example viz -- --seed 445566 --extent 8 --mode walk --out /tmp/smoke
cargo run --release --example walkability -- --seed 445566 --extent 8
cargo run --release --example interiors_gate
cargo bench --bench chunk_gen --no-run
```

## Architecture constraints (from `Urbix_Project.md`)

- **Deterministic generation**: everything derives from `hash(x, y, seed,
  domain)`. No global RNG, no cross-chunk write dependencies. Any new
  generator must follow this.
- **FFI-first / language-agnostic**: public data types are `#[repr(C)]`.
  `include/urbix.h` is auto-generated from `src/ffi.rs` via cbindgen in
  `build.rs` (regenerates on build; check the diff in). Keep public signatures
  C-compatible.
- **Fuzzy Voronoi districts**: a fixed set of seed-derived Voronoi sites
  (24–48) mapped to 5 zone types, queried continuously for zone affinity, not
  a per-chunk static map.
- **Bounded memory**: chunks are LRU-cached and evicted beyond draw distance.

## Conventions

- **Comments**: every public item documented (what/why/how). Placeholder code
  is marked `// TODO(Milestone N)` and must stay clearly marked.
- **Changelog**: keep `CHANGELOG.md` in Keep a Changelog format, SemVer
  `MAJOR.MINOR.PATCH`. Add entries under `[Unreleased]` for any change.
- **Version**: bump in `Cargo.toml` + `CHANGELOG.md` per SemVer rules in
  `Urbix_Project.md` §5.
- `.gitignore` excludes `/target` and `Cargo.lock` (library crate, lockfile
  intentionally untracked).

## Gotchas (learned the hard way)

- **cbindgen excludes**: every new `hash::domain` constant must be added to
  the `exclude` list in `cbindgen.toml`, or it leaks into `include/urbix.h`.
  `build.rs` also carries a manual `ZoneParams` fallback definition and the
  `URBIX_FLAG_*` shims — update both when flags/params change.
- **Wire invariants**: `Cell` is 40 B / header 32 B (compile asserts);
  `CellFlags` bits are additive; `ZoneParams` padding has absorbed new `u8`
  fields so far. `Cell` itself is frozen — grow context/FFI types instead
  (MINOR bumps).
- **Payload growth appends, never interleaves**: new FFI layers go at the
  end so old readers slicing a prefix keep working; pin new layouts with
  `_Static_assert`s in `build.rs`.
- **Schema evolution**: new `#[repr(C)]`/serde fields get `#[serde(default)]`
  so old TOML/JSON files keep parsing; prove it with a strip-and-reparse
  round-trip test (`config.rs`). Validate new knobs in `is_valid`.
- **Flag checks read `cell.flags.contains(FLAG)`**, never the reverse.
- **Seed-agnostic tests**: hashed axes/rotations vary per block, so scan for
  test fixtures (e.g. a same-lot pair) instead of hardcoding coordinates;
  float asserts use range-`contains`.
- **Vendored header**: `3d-explorer-sdk/` pins its own `urbix.h` copy —
  leave it alone unless working on the SDK.

## Git

- Remote: `git@github.com:drax-xard/Urbix.git` (SSH).
- Auth must be done manually: the SSH key is passphrase-protected and not in
  the agent; the user handles `ssh-add` and pushing. An agent should commit
  locally but expect the user to push.
- Git identity is configured **locally** (repo-scoped) as `drax-xard`.
- GitHub workflow: `main`, local commits ahead of `origin/main`. Match commit
  message style in `git log` (short imperative subjects, e.g. "Fill …",
  "Add …").

## Reference files

- `Urbix_Project.md` — design doc: objectives, architecture, wire format,
  FFI surface, milestone plan (§2, §3, §7).
- `README.md` — top-level intent (minimal; keep in sync if expanded).
- `CHANGELOG.md` — versioned history.
- `docs/` — deeper write-ups (`world_generation.md`, `api.md`,
  `interiors.md`, `believable_city.md`); `docs/session_report.md` carries
  cross-session handoff notes when present.
