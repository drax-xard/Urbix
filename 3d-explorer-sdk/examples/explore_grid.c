/* examples/explore_grid.c
 *
 * Fully worked example of driving the Urbix engine and turning a radial
 * grid of chunks into scene-ready geometry data (positions + heights + hues).
 *
 * This is deliberately single-threaded, synchronous, and dependency-free so it
 * is trivial to adapt into a 3D system (replace the printf mesh dump with your
 * renderer's box/instance spawning).
 *
 * Build (macOS / Apple Silicon):
 *   cc -I ../sdk/include explore_grid.c ../sdk/lib/liburbix.a \
 *      -framework Security -framework CoreFoundation -lm -o explore_grid
 * Run:
 *   ./explore_grid
 *
 * Read docs/api.md for the full contract; the world-position formula and cell
 * layout used here are the ones you must match in a renderer.
 */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

#include "urbix.h"

/* ---- Wire layout checks (static; fail at compile time if the header/lib mismatch) ---- */
_Static_assert(sizeof(UrbixChunkHeader) == 32, "UrbixChunkHeader must be 32 bytes");
_Static_assert(sizeof(UrbixCell) == 40,        "UrbixCell must be 40 bytes");
_Static_assert(_Alignof(UrbixCell) == 8,       "UrbixCell must be 8-byte aligned");

/* Dominant zone index = strongest affinity weight. */
static int dominant_zone(const UrbixCell *c) {
    int best = 0;
    for (int z = 1; z < ZONE_COUNT; ++z)
        if (c->zone_affinity[z] > c->zone_affinity[best]) best = z;
    return best;
}

/* Per-zone RGB hues, same values as WorldConfig::default().zone_hues. They are
 * static here so the example stays dependency-free; a real renderer should read
 * them from its WorldConfig (zone_hues[zone][rgb]). */
static const uint8_t ZONE_HUES[ZONE_COUNT][3] = {
    {100, 150, 220}, /* 0 Downtown    */
    { 96, 180,  90}, /* 1 Residential */
    {235, 160,  70}, /* 2 Commercial  */
    {150, 130, 115}, /* 3 Industrial  */
    {140, 205, 120}, /* 4 Park        */
};

int main(void) {
    /* 1. Create the engine (default config + seed). Always check the handle. */
    UrbixEngine *engine = urbix_engine_create(445566u);
    if (!engine) { fprintf(stderr, "urbix_engine_create failed\n"); return 1; }

    /* Keep a 2-chunk radius in cache as we stream. */
    urbix_set_draw_distance(engine, 2);

    /* 2. Generate a 3x3 radial grid of chunks around chunk (0,0) and dump the
     *    mesh-relevant data for every cell. In a real explorer the camera
     *    moves and you regenerate the ring around the current chunk. */
    int radius = 1;
    long spawned_boxes = 0, road_quads = 0;
    for (int cy = -radius; cy <= radius; ++cy) {
        for (int cx = -radius; cx <= radius; ++cx) {
            UrbixChunkBuffer buf = urbix_generate_chunk(engine, cx, cy);
            if (!buf.data || buf.len == 0) {
                fprintf(stderr, "urbix_generate_chunk(%d,%d) failed\n", cx, cy);
                continue;
            }

            const UrbixChunkHeader *hdr  = (const UrbixChunkHeader *)buf.data;
            const UrbixCell        *cells =
                (const UrbixCell *)(buf.data + sizeof(UrbixChunkHeader));

            /* Verify the buffer is header + cell_count cells, no padding. */
            if (buf.len != sizeof(UrbixChunkHeader) +
                           (uint64_t)hdr->cell_count * sizeof(UrbixCell)) {
                fprintf(stderr, "chunk %d,%d: bad buffer length\n", cx, cy);
                urbix_chunk_free(buf);
                continue;
            }

            printf("chunk (%d,%d): %u cells, chunk_size=%u seed=%llu\n",
                   hdr->cx, hdr->cy, hdr->cell_count, hdr->chunk_size,
                   (unsigned long long)hdr->seed);

            for (uint32_t i = 0; i < hdr->cell_count; ++i) {
                const UrbixCell *c = &cells[i];
                /* Cell i of chunk (cx,cy) -> world position (1 unit per cell). */
                double wx = (double)cx * hdr->chunk_size + (double)(i % hdr->chunk_size);
                double wz = (double)cy * hdr->chunk_size + (double)(i / hdr->chunk_size);

                if (c->flags & CellFlags_IS_STREET) {
                    /* Flat road quad at height 0. */
                    printf("  road   x=%6.0f z=%6.0f\n", wx, wz);
                    ++road_quads;
                } else if (c->height > 0.0f) {
                    /* Building box: base (wx, 0, wz), height c->height.
                       Hue = zone_hues of dominant zone; modulate by palette_id. */
                    int z = dominant_zone(c);
                    printf("  build  x=%6.0f z=%6.0f h=%6.1f zone=%d pal=%d hue_rgb=(%u,%u,%u)\n",
                           wx, wz, c->height, z, c->palette_id,
                           ZONE_HUES[z][0], ZONE_HUES[z][1],
                           ZONE_HUES[z][2]);
                    ++spawned_boxes;
                }
            }

            /* IMPORTANT: free EVERY successful buffer here, never with free(). */
            urbix_chunk_free(buf);
        }
    }

    /* 3. Continuous zone query at a world point (useful for ground colour). */
    UrbixZoneAffinity aff = urbix_get_zone(engine, 100.0, 200.0);
    printf("zone affinity @(100,200) = [%.2f %.2f %.2f %.2f %.2f]\n",
           aff.weights[0], aff.weights[1], aff.weights[2],
           aff.weights[3], aff.weights[4]);

    /* 4. Tear down the engine; handle is dangling afterwards. */
    urbix_engine_destroy(engine);

    printf("done: %ld boxes, %ld road quads spawned\n", spawned_boxes, road_quads);
    return 0;
}
