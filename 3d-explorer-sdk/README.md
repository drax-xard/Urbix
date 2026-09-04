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
│   └── explore_grid.c        <-- fully worked example: drive a chunk grid + render data
└── sdk/
    ├── include/urbix.h       <-- C header (all types + functions)
    ├── lib/
    │   ├── liburbix.a        <-- static library
    │   └── liburbix.dylib    <-- dynamic library
    └── urbix-0.8.0-macos-aarch64.tar.gz   <-- CI release artifact (same contents)
```

Target platform of this build: **macOS, Apple Silicon (`aarch64`)**. Version:
**0.8.0**.

## For an AI agent: how to integrate

Follow this order so you build on the right foundation:

1. **Read `docs/api.md`** — it contains the complete FFI contract: types, layout
   of the chunk buffer (`UrbixChunkHeader` + `UrbixCell[]`), every `urbix_*`
   function, ownership/error rules, the `WorldConfig`, and the streaming pattern.
2. **Read `examples/explore_grid.c`** — a small, compile-and-run C program that
   generates a radial grid of chunks and verifies the header/cell layout. Modeling
   your 3D mesh generation directly on `explore_grid.c` is the fastest path.
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

## Compiling your C integration

```sh
cc -I sdk/include your_system.c sdk/lib/liburbix.a \
   -framework Security -framework CoreFoundation -lm \
   -o your_system
```

(If linking the `.dylib`, use `-L sdk/lib -lurbix` and ensure the dylib is
findable at runtime via `DYLD_LIBRARY_PATH` or `install_name_tool`.)

## Chunk buffer memory — read this carefully

`urbix_generate_chunk` returns a buffer whose `data` is Rust-allocated. You must
free it with `urbix_chunk_free(buf)`, **never** with the C `free()`. Each chunk
buffer is independent and safe to pass across an FFI boundary; just free each one
once.

## Building the demo from scratch

1. (Optional) Generate windows/instances: for each cell with `height > 0`, draw a
   box (`wx, 0, wz`) sized `1×height×1`; tint by `zone_hues[dominant_zone]`
   modulated by `palette_id` for variety. For streets (`CellFlags_IS_STREET`),
   draw a flat road quad.
2. Move the camera; track the current chunk `(pcx,pcy)` = `(camera_x/chunk_size,
   camera_z/chunk_size)`; regenerate the radius ring as you cross chunk borders.
3. Control memory with `urbix_set_draw_distance(engine, radius)` — the engine
   evicts chunks beyond it automatically.

See `docs/api.md` §7 and §8 for the exact loops and the world-position formula.

## License & provenance

This SDK packages `urbix` v0.8.0. See `sdk/.../LICENSE` (inside the tarball) and
the engine repo metadata. The header and libs are generated from the Urbix crate.
