# C API Reference — Urbix

Generated from `src/ffi.rs:1` via `cbindgen` (`build.rs`, `cbindgen.toml`) into
`include/urbix.h`. The header is checked in and regenerated on `cargo build`
(best-effort so offline builds keep working). All on-wire types are
`#[repr(C)]` and match `Urbix_Project.md §2.3`.

## Wire Format

```c
typedef struct {
    int32_t  cx;          // chunk column
    int32_t  cy;          // chunk row
    uint32_t cell_count;  // chunk_size * chunk_size
    uint16_t chunk_size;  // e.g. 32
    uint8_t  _pad[6];
    uint64_t seed;
} UrbixChunkHeader; // 32 B, 8-byte aligned

typedef struct {
    float    height;             // 0 = street/open
    float    zone_affinity[5];   // per ZoneType, sum ~1
    uint8_t  palette_id;
    uint8_t  flags;              // bit 0 = is_street, bit 1 = is_park
    uint16_t _pad;
    uint64_t interior_id;        // 0 = none
} UrbixCell; // 40 B, 8-byte aligned
```

`chunk = ChunkHeader + cell_count * UrbixCell`, no padding. Shipped header
inside-guard asserts `sizeof == 32/40` and `align == 8`, plus compat shims
`URBIX_FLAG_STREET`/`URBIX_FLAG_PARK` (alias `CellFlags_IS_*`).

## Lifecycle

```c
#include "urbix.h"

UrbixEngine* urbix_engine_create(uint64_t seed);
void         urbix_engine_destroy(UrbixEngine* e); // null is no-op
```

`UrbixEngine` is opaque (`typedef struct UrbixEngine UrbixEngine;`). Never
dereference; only pass the handle back. Created with `WorldConfig` defaults
(`chunk_size=32`, `draw_distance=8`, `voronoi_site_count` 24–48).

## Chunk Generation

```c
UrbixChunkBuffer urbix_generate_chunk(UrbixEngine* e, int32_t cx, int32_t cy);
void             urbix_chunk_free(UrbixChunkBuffer buf);
```

```c
typedef struct { uint8_t* data; uint64_t len; } UrbixChunkBuffer;
```

`urbix_generate_chunk` returns the buffer **by value** (`{data,len}`) with
`len == sizeof(header) + cell_count*sizeof(cell)`. On null `e`, returns
`{NULL,0}`. The buffer is Rust-allocated; **ownership transfers to the caller**
and must be freed only via `urbix_chunk_free` (never `free`/`delete`). The
free is a no-op on `{NULL,0}` and double-free is UB. Example:

```c
UrbixEngine *e = urbix_engine_create(445566);
UrbixChunkBuffer b = urbix_generate_chunk(e, 0, 0);
UrbixChunkHeader *h = (UrbixChunkHeader*)b.data;
UrbixCell *cells = (UrbixCell*)(b.data + sizeof(UrbixChunkHeader));
for (uint32_t i=0;i<h->cell_count;i++) { /* cells[i].height ... */ }
urbix_chunk_free(b);
urbix_engine_destroy(e);
```

`examples/basic_usage.c:1` is the minimal end-to-end (compiled + linked in
`tests/c_link_run.rs:1` against the `staticlib`).

## Zone Query

```c
typedef struct { float weights[5]; } UrbixZoneAffinity;
UrbixZoneAffinity urbix_get_zone(UrbixEngine* e, double wx, double wz);
```

Returns blended affinity (Shepard, continuous) at world coords `wx/wz` (`double`
to match `WorldEngine::get_zone_affinity(f64,f64)` and stay precise over `i64`
span). `weights` sum ~1. On null `e` returns zeros.

## Configuration

```c
void urbix_set_draw_distance(UrbixEngine* e, uint32_t radius);
void urbix_set_chunk_size(UrbixEngine* e, uint16_t size);
```

* `set_draw_distance` — Chebyshev radius in chunks (null-safe).
* `set_chunk_size` — cells per side for *subsequent* chunks; clears the
  `ChunkCache` so stale-size buffers cannot mix. `size==0` is a no-op at the
  C boundary (Rust `WorldEngine::set_chunk_size` would panic, but FFI must not
  unwind).

Both are null-safe; `set_chunk_size(0)` at the C boundary is deliberately a
no-op to avoid unwinding into the caller (`src/ffi.rs:181`).

## Threading & Versioning

* Engine handle is **not** thread-safe; access from one thread or behind your
  own lock. Generation itself is pure per-chunk (`hash(x,y,seed,domain)`), so a
  worker-pool parallelizing `urbix_generate_chunk` across handles is safe (future
  §8.8).
* `urbix.h` is versioned with the crate (`Cargo.toml` `crate-type = ["lib",
  "staticlib","cdylib"]`, `0.6.0`). No breaking wire change without a minor
  bump.

## Error Handling

* Null `UrbixEngine*` → safe no-op / zero return; never crashes.
* Truncated `UrbixChunkBuffer` → `ChunkBuffer::header()`/`get_cell()` assert and
  refuse OOB (`src/data.rs:234`), so malformed buffers from foreign code are
  caught.
* CLI (`src/main.rs:1`) surfaces errors via `anyhow` with non-zero exit.
