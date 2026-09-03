/* examples/basic_usage.c
 *
 * Minimal C consumer that drives the engine through its FFI:
 * create -> generate a chunk -> read the header and a few cells -> free.
 *
 * Compile (Milestone 5 links this against the engine staticlib):
 *   cc -I include -c examples/basic_usage.c -o /tmp/basic_usage.o
 *
 * The on-wire layout of UrbixChunkHeader / UrbixCell must match their
 * `repr(C)` Rust counterparts; see Urbix_Project.md §2.3.
 */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

#include "urbix.h"

/* Byte layout sanity: header is 32 bytes, a cell is 40 bytes. */
_Static_assert(sizeof(UrbixChunkHeader) == 32, "UrbixChunkHeader size");
_Static_assert(sizeof(UrbixCell) == 40, "UrbixCell size");

/* Offsets into the wire records, from Urbix_Project.md §2.3. */
_Static_assert(_Alignof(UrbixChunkHeader) == 8, "header alignment");
_Static_assert(_Alignof(UrbixCell) == 8, "cell alignment");

int main(void) {
    UrbixEngine *engine = urbix_engine_create(445566u);
    if (!engine) {
        fprintf(stderr, "failed to create engine\n");
        return 1;
    }

    UrbixChunkBuffer buf = urbix_generate_chunk(engine, 1, 2);
    if (!buf.data || buf.len == 0) {
        fprintf(stderr, "failed to generate chunk\n");
        urbix_engine_destroy(engine);
        return 1;
    }

    /* Header lives at the start of the buffer. */
    const UrbixChunkHeader *hdr = (const UrbixChunkHeader *)buf.data;
    if (hdr->cx != 1 || hdr->cy != 2) {
        fprintf(stderr, "unexpected chunk coords %d,%d\n", hdr->cx, hdr->cy);
        urbix_chunk_free(buf);
        urbix_engine_destroy(engine);
        return 1;
    }

    /* The buffer must be exactly header + cell_count cells, no padding. */
    uint64_t expected = sizeof(UrbixChunkHeader) +
                        (uint64_t)hdr->cell_count * sizeof(UrbixCell);
    if (buf.len != expected) {
        fprintf(stderr, "bad buffer length: %llu vs %llu\n",
                (unsigned long long)buf.len, (unsigned long long)expected);
        urbix_chunk_free(buf);
        urbix_engine_destroy(engine);
        return 1;
    }

    /* Scan cells: heights must be >= 0 and finite. */
    const UrbixCell *cells = (const UrbixCell *)(buf.data + sizeof(UrbixChunkHeader));
    for (uint32_t i = 0; i < hdr->cell_count; ++i) {
        if (cells[i].height != cells[i].height || cells[i].height < 0.0f) {
            fprintf(stderr, "bad height at cell %u\n", i);
            urbix_chunk_free(buf);
            urbix_engine_destroy(engine);
            return 1;
        }
    }

    /* Zone query and config setters round out the surface. */
    UrbixZoneAffinity zone = urbix_get_zone(engine, 100.0, 200.0);
    float sum = 0.0f;
    for (int i = 0; i < 5; ++i) sum += zone.weights[i];
    if (sum < 0.99f || sum > 1.01f) {
        fprintf(stderr, "zone weights do not sum to 1: %f\n", sum);
        urbix_chunk_free(buf);
        urbix_engine_destroy(engine);
        return 1;
    }

    urbix_set_draw_distance(engine, 3);
    urbix_set_chunk_size(engine, 16);
    /* Zero size is a documented no-op at the C boundary (no panic across FFI). */
    urbix_set_chunk_size(engine, 0);

    urbix_chunk_free(buf);
    urbix_engine_destroy(engine);

    printf("basic_usage: ok\n");
    return 0;
}
