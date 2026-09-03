/* include/urbix.h
 *
 * C header for the Urbix engine FFI.
 *
 * Declares the opaque engine handle, the `repr(C)` data records
 * (`UrbixCell`, `UrbixChunkHeader`, `UrbixZoneAffinity`, `UrbixChunkBuffer`),
 * and the engine lifecycle / chunk generation / zone query / configuration
 * entry points.
 *
 * Intended to be produced automatically by `cbindgen` from `src/ffi.rs`
 * (wired through `build.rs` in Milestone 5); until then it is maintained by
 * hand and must stay in lockstep with `src/ffi.rs`.
 *
 * See `Urbix_Project.md` §2.3 and §2.4 for the wire format and API reference.
 *
 * MEMORY CONTRACT
 * ---------------
 * - `urbix_engine_create` returns an opaque handle the caller owns; release it
 *   with `urbix_engine_destroy`.
 * - `urbix_generate_chunk` returns a buffer the caller owns; release it with
 *   `urbix_chunk_free`. It must NEVER be freed by a foreign allocator
 *   (`free`, `delete`, ...) — only `urbix_chunk_free`.
 * - All other functions take borrowed state and transfer no ownership.
 */
#ifndef URBIX_H
#define URBIX_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque engine handle. Created by `urbix_engine_create`, destroyed by
 * `urbix_engine_destroy`. Never dereference from C. */
typedef struct UrbixEngine UrbixEngine;

/* Wire header of a chunk: fixed-size, followed by `cell_count` `UrbixCell`s. */
typedef struct UrbixChunkHeader {
    int32_t  cx;          /* chunk column index */
    int32_t  cy;          /* chunk row index */
    uint32_t cell_count;  /* total cells (chunk_size x chunk_size) */
    uint16_t chunk_size;  /* cells per chunk side, e.g. 32 */
    uint8_t  _pad[6];     /* alignment padding */
    uint64_t seed;        /* world seed, for verification */
} UrbixChunkHeader;

/* Cell flags bitfield. */
enum {
    URBIX_FLAG_STREET = 1u << 0,
    URBIX_FLAG_PARK   = 1u << 1,
};

/* A single city cell. */
typedef struct UrbixCell {
    float   height;                 /* building height (0 = street/open) */
    float   zone_affinity[5];       /* weight per zone type */
    uint8_t palette_id;             /* facade color index */
    uint8_t flags;                  /* URBIX_FLAG_* */
    uint16_t _pad;                  /* alignment */
    uint64_t interior_id;           /* deterministic interior key (0 = none) */
} UrbixCell;

/* An owned chunk buffer returned by `urbix_generate_chunk`. */
typedef struct UrbixChunkBuffer {
    uint8_t* data;  /* header followed by cell_count UrbixCell records */
    uint64_t len;   /* sizeof(UrbixChunkHeader) + cell_count * sizeof(UrbixCell) */
} UrbixChunkBuffer;

/* Blended zone-affinity vector, one weight per zone type (sum ~1). */
typedef struct UrbixZoneAffinity {
    float weights[5];
} UrbixZoneAffinity;

/* Lifecycle */
UrbixEngine* urbix_engine_create(uint64_t seed);
void         urbix_engine_destroy(UrbixEngine* engine);

/* Chunk generation - caller must free with `urbix_chunk_free` */
UrbixChunkBuffer urbix_generate_chunk(UrbixEngine* engine, int32_t cx, int32_t cy);
void             urbix_chunk_free(UrbixChunkBuffer buf);

/* Zone query */
UrbixZoneAffinity urbix_get_zone(UrbixEngine* engine, double wx, double wz);

/* Configuration */
void urbix_set_draw_distance(UrbixEngine* engine, uint32_t radius);
void urbix_set_chunk_size(UrbixEngine* engine, uint16_t size);

#ifdef __cplusplus
}
#endif

#endif /* URBIX_H */
