# Running the Urbix examples

The `examples/` folder holds standalone demo binaries (each is its own crate;
they call the public `urbix` API only). None of them ship in the library — on
purpose — so the engine's public surface stays exactly what FFI consumers use.

## Toolchain note

Rust is installed via rustup but is **not on the default shell PATH**. In any
new shell, source it first:

```sh
. "$HOME/.cargo/env"
```

Use `--release` for non-trivial seeds/extents — `hybrid` visualizers chew CPU in
debug builds.

## `cli_demo` — inspect a built cell's interior (headless)

Drives the Milestone-9 bridge (`chunk::interior_context_for` → context,
`WorldConfig::blueprint_for` → rule table, `interior::generate_layout` →
mini-world) and prints a wall-of-text report for one lot: the `InteriorContext`,
the `Blueprint`, and every storey's tile grid with a legend.

```sh
cargo run --release --example cli_demo                 # inspect the tallest built cell in chunk (0,0)
cargo run --release --example cli_demo -- --seed 42 --cx 1 --cy -2
cargo run --release --example cli_demo -- --seed 42 --cx 0 --cy 0 --dx 20 --dy 19
```

Flags (simple `--key value` parser, no `--help`):

- `--seed <u64>` — world seed (default 445566)
- `--cx <i32>` / `--cy <i32>` — chunk column/row (default 0)
- `--dx <u32>` / `--dy <u32>` — local cell inside the chunk; absent → the
  tallest built cell is inspected instead

## `viz` — 2D city visualizer → images

Generates an `extent × extent` grid of chunks and writes `.ppm` + `.png`
files (one pixel per cell). Two colouring modes:

- **hybrid (default)** — zone colour, brightened by building height (tall =
  bright skyline); streets drawn as roads
- **affinity** — flat per-cell dominant-zone map

```sh
cargo run --release --example viz                       # default seed 0, extent 16 → out-*.png
cargo run --release --example viz -- --seed 445566 --extent 32 --mode affinity
cargo run --release --example viz -- --seed 445566 --inspect 120,400   # + interior report for world cell (120,400)
```

Flags:

- `--seed <u64>` — world seed (default 0)
- `--center-cx <i32>` / `--center-cy <i32>` — chunk at the grid centre (default 0)
- `--extent <u32>` — chunks per side (default 16; 16 → 512×512 px)
- `--chunk-size <u16>` — cells per chunk side (default 32)
- `--mode <hybrid|affinity>` — colouring (default hybrid)
- `--out <path>` — output base path (default `out`)
- `--inspect <wx,wz>` — print the interior report for the cell at absolute
  world coordinates `(wx,wz)` after rendering (same output as `cli_demo`)

## `interactive` — streaming explorer (egui window)

An egui/eframe window that pans/zooms over the infinite world, proving bounded
LRU streaming (`generated`/`cache` HUD), and lets you **click a cell** to see
its interior beside the map: zone, per-storey room/corridor stats, a storey
slider, and a live ASCII map of the chosen floor.

```sh
cargo run --release --example interactive
```

Controls: WASD/Arrows pan, wheel zooms, extent/zoom sliders, hybrid/affinity
toggle, click a lot to inspect, "clear selection ✕" to dismiss.

## `basic_usage.c` — the C FFI entry point

Not built by `cargo`. It's a hand-written C consumer of the generated
`include/urbix.h` (regenerated on every `cargo build`). Compile it against the
static C library once you have a C toolchain:

```sh
cc examples/basic_usage.c -I include target/release/liburbix.a -o basic_usage
```

## Related benchmarks

- `cargo bench --bench chunk_gen` — criterion benchmark (single chunk,
  100-sweep, cache hit/miss). It lives under `benches/`, not `examples/`.