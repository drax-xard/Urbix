# Urbix 3D-Explorer SDK

A self-contained, **language-agnostic** release of the Urbix procedural city
engine for building a 3D city explorer/demo. The engine generates an infinite,
deterministic city in chunks; you render it.

## What's here

```
3d-explorer-sdk/
├── README.md                 <-- you are here; agent starting point
├── docs/
│   └── api.md                <-- THE authoritative C API reference (read this first)
├── examples/
│   ├── explore_grid.c        <-- fully worked example: drive a chunk grid + render data
│   ├── explore_interior.c    <-- worked example: read a building's interior via the FFI
│   └── explore_rooms.c       <-- worked example: read per-room records via the FFI
├── server/
│   ├── serve.c               <-- dependency-free C micro-server (links the shipped lib)
│   ├── build.sh              <-- compiles server/serve
│   ├── test.sh               <-- headless HTTP smoke tests
│   └── www/                  <-- Three.js city explorer (a working demo app)
└── sdk/
    ├── include/urbix.h       <-- C header (all types + functions)
    ├── lib/
    │   ├── liburbix.a        <-- static library
    │   └── liburbix.dylib    <-- dynamic library
    └── urbix-0.15.0-macos-aarch64.tar.gz   <-- CI release artifact (same contents)
```

This build vendors **macOS, Apple Silicon (`aarch64`)**. Version:
**0.15.0**. Linux (`linux-x86_64`) and Windows (`windows-x86_64`)
tarballs ship from the same GitHub Release (`release.yml`) with the same
layout — swap `sdk/lib` + `sdk/include` to retarget this SDK.

## For an AI agent: how to integrate

Follow this order so you build on the right foundation:

1. **Read `docs/api.md`** — it contains the complete FFI contract: types, layout
   of the chunk buffer (`UrbixChunkHeader` + `UrbixCell[]`), every `urbix_*`
   function, ownership/error rules, the `WorldConfig`, and the streaming pattern.
2. **Read `examples/explore_grid.c`** — a small, compile-and-run C program that
   generates a radial grid of chunks and verifies the header/cell layout. Modeling
   your 3D mesh generation directly on `explore_grid.c` is the fastest path.
   `examples/explore_interior.c` does the same for a building's interiors.
3. **Write your 3D demo** against the API in `docs/api.md`. The conceptual flow:

   ```text
   engine = urbix_engine_create(seed)
   for each chunk (cx,cy) within render radius around the camera:
       buf = urbix_generate_chunk(engine, cx, cy)
       read header + cells -> spawn boxes/instances per cell
       urbix_chunk_free(buf)
   urbix_engine_destroy(engine)
   ```

### The one idea to internalize

A **chunk** is a square of `chunk_size × chunk_size` cells. Each **cell** carry
a building height, zone affinity, palette id, and flags — enough to place a box
or an instance at world position `(cx*chunk_size + i%chunk_size, cy*chunk_size + i/chunk_size)`.
The engine is deterministic from a seed and streams infinitely; cache eviction
is automatic. You don't persist anything — just generate on demand.

Since 0.10.0 the engine also exposes what's **inside** each built cell:
`urbix_generate_interior(engine, wx, wz)` returns per-storey tile grids
(`UrbixInterior`: walls, doors, the circulation core, corridors, rooms
tagged by kind, plus a parallel furniture layer since 0.15.0) so a viewer
can render room layouts — good enough for a fly-through demo. Since 0.15.0
`urbix_generate_interior_rooms(engine, wx, wz)` also returns per-room
records (`UrbixRoom`: floor, rect, kind, apartment unit, area) that always
agree with the grids. See `docs/api.md` §5.

## Compiling your C integration

```sh
# macOS (Apple Silicon)
cc -I sdk/include your_system.c sdk/lib/liburbix.a \
   -framework Security -framework CoreFoundation -lm \
   -o your_system
# Linux
cc -I sdk/include your_system.c sdk/lib/liburbix.a \
   -ldl -lm -pthread \
   -o your_system
# Windows (MinGW)
cc -I sdk/include your_system.c sdk/lib/urbix.lib \
   -lws2_32 -luserenv \
   -o your_system.exe
```

(If linking the `.dylib`/`.so`, use `-L sdk/lib -lurbix` and ensure the
shared lib is findable at runtime via `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH`
or `install_name_tool`. The `urbix-*.tar.gz` artifacts in GitHub Releases
carry the same layout per target — see `sdk/urbix-0.15.0-*.tar.gz`.)

## Chunk buffer memory — read this carefully

`urbix_generate_chunk` returns a buffer whose `data` is Rust-allocated. You must
free it with `urbix_chunk_free(buf)`, **never** with the C `free()`. Each chunk
buffer is independent and safe to pass across an FFI boundary; just free each one
once.

## Building the demo from scratch

1. (Optional) Generate windows/instances: for each cell with `height > 0`, draw a
   box (`wx, 0, wz`) sized `1×height×1` (scale canon: 1 cell = 4 m, so scale
   positions by 4 for metres); tint by `zone_hues[dominant_zone]` modulated
   by `palette_id` for variety. For streets (`CellFlags_IS_STREET`), draw a
   flat road quad — arterials wider/brighter, sidewalks pale, plazas warm,
   greenways green (see `docs/api.md` §3 for the full flag table).
2. Move the camera; track the current chunk `(pcx,pcy)` = `(camera_x/chunk_size,
   camera_z/chunk_size)`; regenerate the radius ring as you cross chunk borders.
3. Control memory with `urbix_set_draw_distance(engine, radius)` — the engine
   evicts chunks beyond it automatically.

See `docs/api.md` §7 and §8 for the exact loops and the world-position formula.

## Running the web explorer (Three.js)

This SDK ships a working demo: a dependency-free C micro-server that links the
shipped library and serves JSON plus a Three.js single-page explorer.

```sh
./server/build.sh                 # compiles server/serve (macOS/Linux/MinGW)
./server/serve --seed 445566 --web server/www   # default port 8311
# open http://localhost:8311
# optional tuning:
./server/serve --seed 7 --chunk-size 32 --draw-distance 8 --web server/www
```

Controls: drag to orbit, scroll to zoom. Look at a building and click it (or
press G/Enter) to fade into its interior — the exterior fades out so you never
see both at once. Inside, interior walls block your movement (no clipping out
through the facade); WASD/arrows move, Q/E turn, R/F change storey, mouse-look
on click, G or Esc fades back out to the same orbit view. The camera stays
level with the ground and floats at pedestrian eye height. Press V for the
walkability overlay (dark building masses, high-contrast paving) and T to
teleport the orbit target to a world cell. The page pulls `/api/chunks`
(one batch per frame around the camera) and `/api/interior` + `/api/rooms`
on entry, so memory stays bounded; the HUD shows frame time and fetch stats.
Exteriors render the full street hierarchy (arterials, sidewalks, plazas,
greenways) and interiors show furniture, window glazing, and room/unit stats.

To stop the demo, press Shift+Esc (confirm the dialog): the page asks the
server to shut down via `/api/shutdown` and then closes itself (browsers only
auto-close script-opened tabs, so otherwise it shows a "server stopped" screen
you can dismiss).

Endpoints: `/api/config` (seed/settings/zones), `/api/chunk?cx&cy`,
`/api/chunks?cx&cy&r=` (batch square for streaming), `/api/interior?wx&wz`
(tiles+kinds+furniture), `/api/rooms?wx&wz` (room records), `/api/zone?wx&wz`,
static files from `--web`. The server binds localhost only. `three.js` (r160)
is vendored under `server/www/vendor/`, served locally — no external network
access needed.

`server/test.sh` runs headless HTTP smoke tests against the flag defaults;
it needs a localhost connection only.

## License & provenance

This SDK packages `urbix` v0.15.0. See `sdk/.../LICENSE` (inside the tarball) and
the engine repo metadata. The header and libs are generated from the Urbix crate
(`cargo build --release` regenerates `include/urbix.h` via cbindgen).
