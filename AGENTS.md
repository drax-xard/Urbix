# AGENTS.md

Guidance for working in this repository.

## Project state

Urbix is an early-stage Rust crate (v0.1.0) for a deterministic, infinite
procedural city engine. **All `src/*.rs` modules are placeholders** — they
contain only doc-comment headers and `// TODO(Milestone N)` markers. There is
no implemented generation logic yet. Do not search for algorithms that don't
exist; read `Urbix_Project.md` §7 for the milestone roadmap instead.

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

## Architecture constraints (from `Urbix_Project.md`)

- **Deterministic generation**: everything derives from `hash(x, y, seed,
  domain)`. No global RNG, no cross-chunk write dependencies. Any new
  generator must follow this.
- **FFI-first / language-agnostic**: public data types are `#[repr(C)]`.
  `include/urbix.h` is auto-generated from `src/ffi.rs` via cbindgen (not yet
  wired in `build.rs`). Keep public signatures C-compatible.
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
