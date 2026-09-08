/* server/serve.c
 *
 * Dependency-free C micro-server for the Urbix 3D web viewer.
 *
 * Links the shipped C SDK (sdk/lib/liburbix.a) and serves two kinds of
 * response:
 *
 *   /api/...    JSON from the engine (deterministic city data)
 *   /           the static Three.js viewer (index.html + app.js + vendor/)
 *
 * It deliberately has no external dependencies: no libcurl, no JSON library,
 * single-threaded, blocking sockets — sized for a localhost demo (a couple of
 * hundred QPS at most). Hand-rolled JSON is fine here because every number is
 * emitted by us, in a fixed order, from the engine's deterministic output.
 *
 * Build (from 3d-explorer-sdk/):
 *   cc -O2 -I sdk/include server/serve.c sdk/lib/liburbix.a \
 *      -framework Security -framework CoreFoundation -lm -o server/serve
 * Run (from 3d-explorer-sdk/):
 *   ./server/serve [--port 8311] [--seed 445566] [--web server/www]
 *
 * Endpoints:
 *   GET /api/config               { seed, chunk_size, draw_distance, floor_height,
 *                                    zone_hues[5][3], zone_names[5] }
 *   GET /api/chunk?cx=&cy=        { header + cells:[ {x,z,h,zone,pal,flags,interior_id} ] }
 *   GET /api/interior?wx=&wz=     { header + floors:[ {w,d,tiles[],kinds[]} ] }
 *   GET /api/zone?wx=&wz=         { weights:[5] }  (continuous zone affinity)
 *   GET /api/shutdown             { shutting_down:true } then the server exits
 */
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <unistd.h>

#include "urbix.h"

/* ---- Engine config exposed to the viewer ---- */
#define DEFAULT_PORT        8311
#define DEFAULT_SEED        445566u
#define DEFAULT_WEB_DIR     "www"
#define REQ_BUF             8192        /* request line + headers only */
#define MAX_RESPONSE        (128u * 1024u * 1024u) /* guard against runaway */

/* Per-zone RGB hues, byte-identical to WorldConfig::default().zone_hues.
 * Kept static so the server stays dependency-free; the viewer reads them from
 * /api/config rather than hardcoding them. */
static const uint8_t ZONE_HUES[ZONE_COUNT][3] = {
    {100, 150, 220}, /* 0 Downtown    */
    { 96, 180,  90}, /* 1 Residential */
    {235, 160,  70}, /* 2 Commercial  */
    {150, 130, 115}, /* 3 Industrial  */
    {140, 205, 120}, /* 4 Park        */
};
static const char *ZONE_NAMES[ZONE_COUNT] = {
    "downtown", "residential", "commercial", "industrial", "park",
};

/* ---- Growable JSON buffer (dependency-free, fprintf-style appends) ---- */
typedef struct {
    char  *p;
    size_t n;
    size_t cap;
} Jbuf;

/* Set by /api/shutdown so the accept loop can end gracefully. */
static volatile sig_atomic_t g_shutdown = 0;

static void jbuf_reserve(Jbuf *b, size_t extra) {
    if (b->n + extra + 1 > b->cap) {
        size_t nc = b->cap ? b->cap * 2 : 4096;
        while (b->n + extra + 1 > nc) nc *= 2;
        char *np = (char *)realloc(b->p, nc);
        if (!np) { fprintf(stderr, "out of memory\n"); exit(1); }
        b->p = np;
        b->cap = nc;
    }
}

static void jprintf(Jbuf *b, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static void jprintf(Jbuf *b, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    va_list cp;
    va_copy(cp, ap);
    int need = vsnprintf(NULL, 0, fmt, cp);
    va_end(cp);
    if (need < 0) { va_end(ap); return; }
    jbuf_reserve(b, (size_t)need);
    vsnprintf(b->p + b->n, (size_t)need + 1, fmt, ap);
    va_end(ap);
    b->n += (size_t)need;
}

/* ---- Tiny query-string parser (name=value&... pairs) ---- */
static const char *next_pair(const char *q, char *key, size_t kcap, char *val, size_t vcap) {
    if (!q || *q == '\0') return NULL;
    const char *amp = strchr(q, '&');
    size_t seg = amp ? (size_t)(amp - q) : strlen(q);
    char pair[128];
    if (seg >= sizeof(pair)) return NULL;
    memcpy(pair, q, seg);
    pair[seg] = '\0';
    const char *eq = strchr(pair, '=');
    if (eq) {
        size_t kl = (size_t)(eq - pair);
        if (kl + 1 > kcap) kl = kcap - 1;
        memcpy(key, pair, kl);
        key[kl] = '\0';
        snprintf(val, vcap, "%s", eq + 1);
    } else {
        snprintf(key, kcap, "%s", pair);
        val[0] = '\0';
    }
    return amp ? amp + 1 : NULL;
}

static int64_t parse_i64(const char *s, int *ok) {
    errno = 0;
    char *end = NULL;
    long long v = strtoll(s, &end, 10);
    *ok = !errno && end != s;
    return (int64_t)v;
}

static double parse_f64(const char *s, int *ok) {
    errno = 0;
    char *end = NULL;
    double v = strtod(s, &end);
    *ok = errno == 0 && end != s;
    return v;
}

static int query_i(const char *q, const char *want, int32_t *out) {
    char key[16], val[64];
    const char *rest = q;
    while (rest) {
        rest = next_pair(rest, key, sizeof(key), val, sizeof(val));
        if (strcmp(key, want) == 0) {
            int ok = 0;
            int64_t v = parse_i64(val, &ok);
            if (ok) { *out = (int32_t)v; return 1; }
            return 0;
        }
    }
    return 0;
}

static int query_f(const char *q, const char *want, double *out) {
    char key[16], val[64];
    const char *rest = q;
    while (rest) {
        rest = next_pair(rest, key, sizeof(key), val, sizeof(val));
        if (strcmp(key, want) == 0) {
            int ok = 0;
            *out = parse_f64(val, &ok);
            return ok;
        }
    }
    return 0;
}

/* ---- HTTP response plumbing ---- */
static void send_json(int fd, const char *body) {
    char head[512];
    int n = snprintf(head, sizeof(head),
                     "HTTP/1.1 200 OK\r\n"
                     "Content-Type: application/json\r\n"
                     "Connection: close\r\n"
                     "Access-Control-Allow-Origin: *\r\n"
                     "Cache-Control: no-cache\r\n"
                     "Content-Length: %zu\r\n"
                     "\r\n",
                     strlen(body));
    (void)!write(fd, head, (size_t)n);
    (void)!write(fd, body, strlen(body));
}

static void send_error(int fd, int code, const char *reason, const char *msg) {
    char body[256];
    snprintf(body, sizeof(body), "%s\n", msg);
    char head[512];
    int n = snprintf(head, sizeof(head),
                     "HTTP/1.1 %d %s\r\n"
                     "Content-Type: text/plain\r\n"
                     "Connection: close\r\n"
                     "Content-Length: %zu\r\n"
                     "\r\n",
                     code, reason, strlen(body));
    (void)!write(fd, head, (size_t)n);
    (void)!write(fd, body, strlen(body));
}

/* ---- Static files (the Three.js viewer) ---- */
static const char *mime_for(const char *path) {
    const char *dot = strrchr(path, '.');
    if (!dot) return "application/octet-stream";
    if (strcmp(dot, ".html") == 0) return "text/html; charset=utf-8";
    if (strcmp(dot, ".js") == 0)   return "text/javascript; charset=utf-8";
    if (strcmp(dot, ".css") == 0)  return "text/css; charset=utf-8";
    return "application/octet-stream";
}

static int path_escaping(const char *p) {
    return strstr(p, "../") != NULL || strstr(p, "/..") != NULL ||
           strstr(p, "..\\") != NULL || strstr(p, "\\..") != NULL;
}

static void serve_file(int fd, const char *web_dir, const char *path) {
    /* "/" -> index.html; reject anything escaping the web dir. */
    char clean[512];
    const char *name = (strcmp(path, "/") == 0) ? "/index.html" : path;
    if (path_escaping(name)) { send_error(fd, 403, "Forbidden", "forbidden path"); return; }

    int n = snprintf(clean, sizeof(clean), "%s%s", web_dir, name);
    if (n < 0 || (size_t)n >= sizeof(clean)) { send_error(fd, 414, "URI Too Long", "path too long"); return; }

    FILE *f = fopen(clean, "rb");
    if (!f) { send_error(fd, 404, "Not Found", "not found"); return; }

    struct stat st;
    if (stat(clean, &st) != 0 || st.st_size < 0 || (uint64_t)st.st_size > MAX_RESPONSE) {
        fclose(f);
        send_error(fd, 404, "Not Found", "not found");
        return;
    }
    size_t len = (size_t)st.st_size;
    char buf[8192];
    char head[512];
    int hn = snprintf(head, sizeof(head),
                      "HTTP/1.1 200 OK\r\n"
                      "Content-Type: %s\r\n"
                      "Connection: close\r\n"
                      "Content-Length: %zu\r\n"
                      "\r\n",
                      mime_for(clean), len);
    (void)!write(fd, head, (size_t)hn);
    size_t rem = len;
    while (rem > 0) {
        size_t got = fread(buf, 1, rem < sizeof(buf) ? rem : sizeof(buf), f);
        if (got == 0) break;
        (void)!write(fd, buf, got);
        rem -= got;
    }
    fclose(f);
}

/* ---- JSON endpoints ---- */
static void json_config(Jbuf *b, uint64_t seed) {
    jprintf(b, "{\"version\":1,\"seed\":%llu", (unsigned long long)seed);
    jprintf(b, ",\"chunk_size\":32,\"draw_distance\":8,\"floor_height\":4.0");
    jprintf(b, ",\"zone_hues\":[");
    for (int z = 0; z < ZONE_COUNT; ++z) {
        jprintf(b, "%s[%u,%u,%u]", z ? "," : "",
                ZONE_HUES[z][0], ZONE_HUES[z][1], ZONE_HUES[z][2]);
    }
    jprintf(b, "],\"zone_names\":[");
    for (int z = 0; z < ZONE_COUNT; ++z) {
        jprintf(b, "%s\"%s\"", z ? "," : "", ZONE_NAMES[z]);
    }
    jprintf(b, "]}");
}

/* Interior payload layout (UrbixInterior.data, per floor):
 *   tiles[W*D] then kinds[W*D], row-major. */
static void json_interior(Jbuf *b, int32_t wx, int32_t wz, UrbixEngine *e) {
    UrbixInterior in = urbix_generate_interior(e, wx, wz);
    if (in.data == NULL || in.len == 0) {
        jprintf(b, "{\"version\":1,\"wx\":%d,\"wz\":%d,\"floor_count\":0,"
                   "\"footprint_w\":0,\"footprint_d\":0,\"interior_id\":\"0\","
                   "\"floors\":[]}", wx, wz);
        return;
    }
    uint64_t grid = (uint64_t)in.footprint_w * in.footprint_d;
    jprintf(b, "{\"version\":1,\"wx\":%d,\"wz\":%d,\"zone\":%u,\"door_side\":%u,"
               "\"footprint_w\":%u,\"footprint_d\":%u,\"floor_count\":%u,"
               "\"interior_id\":\"%llu\",\"floors\":[",
            wx, wz, in.zone, in.door_side, in.footprint_w, in.footprint_d,
            in.floor_count, (unsigned long long)in.interior_id);
    const uint8_t *p = in.data;
    for (uint32_t f = 0; f < in.floor_count; ++f) {
        jprintf(b, "%s{\"w\":%u,\"d\":%u,\"tiles\":[", f ? "," : "",
                in.footprint_w, in.footprint_d);
        for (uint64_t i = 0; i < grid; ++i) {
            jprintf(b, "%s%u", i ? "," : "", p[i]);
        }
        jprintf(b, "],\"kinds\":[");
        for (uint64_t i = 0; i < grid; ++i) {
            jprintf(b, "%s%u", i ? "," : "", p[grid + i]);
        }
        jprintf(b, "]}");
        p += 2 * grid;
    }
    jprintf(b, "]}");
    urbix_interior_free(in);
}

static void json_chunk(Jbuf *b, int32_t cx, int32_t cy, UrbixEngine *e) {
    UrbixChunkBuffer buf = urbix_generate_chunk(e, cx, cy);
    if (buf.data == NULL || buf.len == 0) {
        jprintf(b, "{\"version\":1,\"cx\":%d,\"cy\":%d,\"cell_count\":0,\"cells\":[]}",
                cx, cy);
        return;
    }
    const UrbixChunkHeader *hdr = (const UrbixChunkHeader *)buf.data;
    const UrbixCell *cells =
        (const UrbixCell *)(buf.data + sizeof(UrbixChunkHeader));
    jprintf(b, "{\"version\":1,\"cx\":%d,\"cy\":%d,\"chunk_size\":%u,"
               "\"seed\":%llu,\"cell_count\":%u,\"cells\":[",
            hdr->cx, hdr->cy, hdr->chunk_size, (unsigned long long)hdr->seed,
            hdr->cell_count);
    for (uint32_t i = 0; i < hdr->cell_count; ++i) {
        const UrbixCell *c = &cells[i];
        int32_t wx = hdr->cx * (int32_t)hdr->chunk_size + (int32_t)(i % hdr->chunk_size);
        int32_t wz = hdr->cy * (int32_t)hdr->chunk_size + (int32_t)(i / hdr->chunk_size);
        int z = 0;
        for (int zi = 1; zi < ZONE_COUNT; ++zi)
            if (c->zone_affinity[zi] > c->zone_affinity[z]) z = zi;
        jprintf(b, "%s{\"x\":%d,\"z\":%d,\"h\":%.1f,\"zone\":%d,\"pal\":%u,"
                   "\"flags\":%u,\"interior_id\":\"%llu\"}",
                i ? "," : "", wx, wz, (double)c->height, z, c->palette_id,
                (unsigned)c->flags, (unsigned long long)c->interior_id);
    }
    jprintf(b, "]}");
    urbix_chunk_free(buf);
}

static void json_zone(Jbuf *b, double wx, double wz, UrbixEngine *e) {
    UrbixZoneAffinity aff = urbix_get_zone(e, wx, wz);
    jprintf(b, "{\"wx\":%.2f,\"wz\":%.2f,\"weights\":[", wx, wz);
    for (int z = 0; z < ZONE_COUNT; ++z) {
        jprintf(b, "%s%.3f", z ? "," : "", (double)aff.weights[z]);
    }
    jprintf(b, "]}");
}

static void json_shutdown(Jbuf *b) {
    jprintf(b, "{\"shutting_down\":true}");
}

/* ---- Request dispatch ---- */
static void handle_request(int fd, const char *req_raw, const char *web_dir,
                           UrbixEngine *e, uint64_t seed) {
    char method[8], path[512], version[16];
    if (sscanf(req_raw, "%7s %511s %15s", method, path, version) != 3) {
        send_error(fd, 400, "Bad Request", "malformed request line");
        return;
    }
    if (strcmp(method, "GET") != 0) {
        send_error(fd, 405, "Method Not Allowed", "GET only");
        return;
    }

    /* Split "path?query". */
    char *q = strchr(path, '?');
    if (q) *q++ = '\0';

    Jbuf b = {0};
    if (strcmp(path, "/api/config") == 0) {
        json_config(&b, seed);
    } else if (strcmp(path, "/api/chunk") == 0) {
        int32_t cx = 0, cy = 0;
        if (!query_i(q, "cx", &cx) || !query_i(q, "cy", &cy)) {
            send_error(fd, 400, "Bad Request", "chunk needs cx=&cy=");
            free(b.p);
            return;
        }
        json_chunk(&b, cx, cy, e);
    } else if (strcmp(path, "/api/interior") == 0) {
        int32_t wx = 0, wz = 0;
        if (!query_i(q, "wx", &wx) || !query_i(q, "wz", &wz)) {
            send_error(fd, 400, "Bad Request", "interior needs wx=&wz=");
            free(b.p);
            return;
        }
        json_interior(&b, wx, wz, e);
    } else if (strcmp(path, "/api/zone") == 0) {
        double wx = 0.0, wz = 0.0;
        if (!query_f(q, "wx", &wx) || !query_f(q, "wz", &wz)) {
            send_error(fd, 400, "Bad Request", "zone needs wx=&wz=");
            free(b.p);
            return;
        }
        json_zone(&b, wx, wz, e);
    } else if (strcmp(path, "/api/shutdown") == 0) {
        json_shutdown(&b);
        g_shutdown = 1;
    } else if (strncmp(path, "/api/", 5) == 0) {
        send_error(fd, 404, "Not Found", "unknown api endpoint");
        free(b.p);
        return;
    } else {
        /* Static viewer files. */
        serve_file(fd, web_dir, path);
        return;
    }

    send_json(fd, b.p ? b.p : "");
    free(b.p);
}

/* ---- entry point ---- */
int main(int argc, char **argv) {
    int  port  = DEFAULT_PORT;
    unsigned long seed = DEFAULT_SEED;
    const char *web_dir = DEFAULT_WEB_DIR;

    for (int i = 1; i < argc - 1; i += 2) {
        const char *flag = argv[i];
        if (strcmp(flag, "--port") == 0) {
            port = atoi(argv[i + 1]);
            if (port <= 0 || port > 65535) { fprintf(stderr, "bad --port\n"); return 2; }
        } else if (strcmp(flag, "--seed") == 0) {
            char *end = NULL;
            seed = strtoul(argv[i + 1], &end, 10);
            if (!end || *end) { fprintf(stderr, "bad --seed\n"); return 2; }
        } else if (strcmp(flag, "--web") == 0) {
            web_dir = argv[i + 1];
        } else {
            fprintf(stderr, "unknown flag %s\n", flag);
            return 2;
        }
    }

    UrbixEngine *engine = urbix_engine_create((uint64_t)seed);
    if (!engine) { fprintf(stderr, "urbix_engine_create failed\n"); return 1; }
    urbix_set_draw_distance(engine, 8);

    signal(SIGPIPE, SIG_IGN);

    int srv = socket(AF_INET, SOCK_STREAM, 0);
    if (srv < 0) { perror("socket"); return 1; }
    int one = 1;
    setsockopt(srv, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons((uint16_t)port);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK); /* localhost only */
    if (bind(srv, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        perror("bind");
        return 1;
    }
    if (listen(srv, 8) != 0) {
        perror("listen");
        return 1;
    }

    fprintf(stderr, "urbix serve: seed=%llu port=%d web=%s\n",
            (unsigned long long)seed, port, web_dir);
    fprintf(stderr, "  open http://localhost:%d in a browser\n", port);

    for (;;) {
        int cfd = accept(srv, NULL, NULL);
        if (cfd < 0) { if (errno == EINTR) continue; perror("accept"); break; }

        char req[REQ_BUF];
        ssize_t got = recv(cfd, req, sizeof(req) - 1, 0);
        if (got > 0) {
            req[got] = '\0';
char *end = strstr(req, "\r\n");
        if (end) *end = '\0';   /* just the request line is all we need */
        handle_request(cfd, req, web_dir, engine, seed);
    }
    close(cfd);
    if (g_shutdown) {
        fprintf(stderr, "urbix serve: shutdown requested, exiting\n");
        break;
    }
}

    urbix_engine_destroy(engine);
    return 0;
}