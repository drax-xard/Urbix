/* examples/explore_rooms.c
 *
 * Worked example of reading per-room records through the M15 FFI: pick a
 * built cell from a chunk, request its room list by world coordinates with
 * urbix_generate_interior_rooms, and print every room (floor, rect, kind,
 * apartment unit, area). The records always agree with the grids from
 * urbix_generate_interior — same engine path, same interior cache.
 *
 * Build (macOS / Apple Silicon):
 *   cc -I ../sdk/include explore_rooms.c ../sdk/lib/liburbix.a \
 *      -framework Security -framework CoreFoundation -lm -o explore_rooms
 * Linux:
 *   cc -I ../sdk/include explore_rooms.c ../sdk/lib/liburbix.a \
 *      -ldl -lm -pthread -o explore_rooms
 * Run:
 *   ./explore_rooms
 *
 * Payload: UrbixRoomList { data, count } — count packed UrbixRoom records
 * (10 B each, row-major scan order). Free exactly once with
 * urbix_interior_rooms_free, never with free(). See docs/api.md §5.
 */
#include <stdio.h>
#include <stdint.h>

#include "urbix.h"

/* ---- Wire layout check: room records are 10 B packed. ---- */
_Static_assert(sizeof(UrbixRoom) == 10, "UrbixRoom must be 10 bytes");

int main(void) {
    UrbixEngine *engine = urbix_engine_create(445566u);
    if (!engine) { fprintf(stderr, "urbix_engine_create failed\n"); return 1; }

    UrbixChunkBuffer buf = urbix_generate_chunk(engine, 0, 0);
    if (!buf.data) { fprintf(stderr, "urbix_generate_chunk failed\n"); return 1; }
    const UrbixChunkHeader *hdr = (const UrbixChunkHeader *)buf.data;
    const UrbixCell *cells = (const UrbixCell *)(buf.data + sizeof(UrbixChunkHeader));

    /* Pick the first built (height > 0) cell in chunk (0,0). */
    int32_t wx = 0, wz = 0;
    int found = 0;
    for (uint32_t i = 0; i < hdr->cell_count; ++i) {
        if (cells[i].height > 0.0f) {
            wx = hdr->cx * (int32_t)hdr->chunk_size + (int32_t)(i % hdr->chunk_size);
            wz = hdr->cy * (int32_t)hdr->chunk_size + (int32_t)(i / hdr->chunk_size);
            found = 1;
            break;
        }
    }
    urbix_chunk_free(buf);
    if (!found) { fprintf(stderr, "no built cell in chunk (0,0)\n"); return 1; }

    UrbixRoomList rooms = urbix_generate_interior_rooms(engine, wx, wz);
    printf("rooms of (%d,%d): count=%llu\n", wx, wz, (unsigned long long)rooms.count);
    for (uint64_t i = 0; i < rooms.count; ++i) {
        const UrbixRoom *r = &rooms.data[i];
        /* unit 255 = outside every apartment (circulation-adjacent room). */
        printf("  room %4llu: floor=%u rect=(%u,%u %ux%u) kind=%u unit=%u area=%u\n",
               (unsigned long long)i, r->floor, r->x, r->z, r->w, r->d,
               r->kind, r->unit, r->area);
    }

    /* IMPORTANT: free exactly once, never with free(). */
    urbix_interior_rooms_free(rooms);
    urbix_engine_destroy(engine);
    return 0;
}
