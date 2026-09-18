/* examples/explore_interior.c
 *
 * Worked example of reading a building's interior through the FFI: pick a
 * built cell from a chunk, request its interior by world coordinates, and
 * print the per-storey tile grids (walls / doors / core / corridor / rooms
 * with kinds + furniture).
 *
 * Build (macOS / Apple Silicon):
 *   cc -I ../sdk/include explore_interior.c ../sdk/lib/liburbix.a \
 *      -framework Security -framework CoreFoundation -lm -o explore_interior
 * Linux:
 *   cc -I ../sdk/include explore_interior.c ../sdk/lib/liburbix.a \
 *      -ldl -lm -pthread -o explore_interior
 * Run:
 *   ./explore_interior
 *
 * The payload layout (UrbixInterior.data) is documented in docs/api.md §5:
 * per floor, tiles[W*D] (Tile enum bytes) then kinds[W*D] (room-kind tags)
 * then furn[W*D] (furniture codes, 0 = bare; M15, appended so readers
 * slicing the first two thirds keep working).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "urbix.h"

static const char *tile_name(uint8_t t) {
    switch (t) {
        case 0: return "void";
        case 1: return "wall";
        case 2: return "door";
        case 3: return "core";
        case 4: return "corridor";
        case 5: return "room";
        default: return "?";
    }
}

int main(void) {
    UrbixEngine *engine = urbix_engine_create(445566u);
    if (!engine) { fprintf(stderr, "urbix_engine_create failed\n"); return 1; }

    UrbixChunkBuffer buf = urbix_generate_chunk(engine, 0, 0);
    if (!buf.data) { fprintf(stderr, "urbix_generate_chunk failed\n"); return 1; }
    const UrbixChunkHeader *hdr = (const UrbixChunkHeader *)buf.data;
    const UrbixCell *cells = (const UrbixCell *)(buf.data + sizeof(UrbixChunkHeader));

    /* Pick the first built (height > 0) cell in chunk (0,0). */
    int32_t wx = 0, wz = 0;
    for (uint32_t i = 0; i < hdr->cell_count; ++i) {
        if (cells[i].height > 0.0f) {
            wx = hdr->cx * (int32_t)hdr->chunk_size + (int32_t)(i % hdr->chunk_size);
            wz = hdr->cy * (int32_t)hdr->chunk_size + (int32_t)(i / hdr->chunk_size);
            break;
        }
    }
    urbix_chunk_free(buf);

    UrbixInterior in = urbix_generate_interior(engine, wx, wz);
    if (!in.data || in.len == 0) {
        fprintf(stderr, "cell (%d,%d) has no interior\n", wx, wz);
        urbix_interior_free(in);
        urbix_engine_destroy(engine);
        return 1;
    }

    uint64_t grid = (uint64_t)in.footprint_w * in.footprint_d;
    printf("interior of (%d,%d): id=%llu zone=%u door_side=%u "
           "footprint=%ux%u floors=%u\n",
           wx, wz, (unsigned long long)in.interior_id, in.zone, in.door_side,
           in.footprint_w, in.footprint_d, in.floor_count);
    if (in.len != (uint64_t)in.floor_count * 3ULL * grid) {
        fprintf(stderr, "bad interior length %llu (expect floors*3*W*D = %llu)\n",
                (unsigned long long)in.len,
                (unsigned long long)in.floor_count * 3ULL * grid);
        urbix_interior_free(in);
        urbix_engine_destroy(engine);
        return 1;
    }
    printf("  len=%llu (= floors*3*W*D: tiles+kinds+furn)\n",
           (unsigned long long)in.len);

    const uint8_t *p = in.data;
    for (uint16_t f = 0; f < in.floor_count; ++f) {
        unsigned rooms = 0, furnished = 0;
        for (uint64_t i = 0; i < grid; ++i) {
            if (p[i] == 5) ++rooms;
            if (p[2 * grid + i] != 0) {
                ++furnished;
                if (p[i] != 5)
                    printf("  floor %u: WARNING furn on non-room tile %llu\n", f,
                           (unsigned long long)i);
            }
        }
        printf("  floor %2u: %u room tiles, %u furnished  |  ", f, rooms, furnished);
        /* Print the first row of tiles so the run output is self-describing. */
        for (uint64_t x = 0; x < in.footprint_w; ++x)
            printf("%c", tile_name(p[x])[0]);
        printf("  |  kinds[0..]=%u,%u,%u furn[0..]=%u,%u,%u\n",
               p[grid], p[grid + 1], p[grid + 2],
               p[2 * grid], p[2 * grid + 1], p[2 * grid + 2]);
        p += 3 * grid;
    }

    urbix_interior_free(in);
    urbix_engine_destroy(engine);
    return 0;
}