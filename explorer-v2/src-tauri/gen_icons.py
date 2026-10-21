#!/usr/bin/env python3
"""Generate the Urbix Explorer icon set from scratch (stdlib only).

Tauri's bundler wants a set of PNGs plus .icns/.ico. We don't ship design
assets in the repo, so this script synthesises a simple "city skyline on a
dark blue field" icon deterministically and then uses macOS `sips`/`iconutil`
(and a hand-rolled ICO writer) to emit every format the config references.

Run from anywhere:  python3 gen_icons.py
"""
import math
import os
import struct
import subprocess
import sys
import zlib

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "icons")
BASE = 1024

# Deterministic pseudo-random skyline (no external deps).
def rng(seed):
    state = seed & 0xFFFFFFFF
    def nxt():
        nonlocal state
        state = (1103515245 * state + 12345) & 0x7FFFFFFF
        return state / 0x7FFFFFFF
    return nxt


def lerp(a, b, t):
    return int(round(a + (b - a) * t))


def make_base():
    """Return a BASE x BASE RGB pixel buffer (list of rows of (r,g,b))."""
    top = (10, 36, 64)
    bottom = (201, 214, 223)
    px = [[(0, 0, 0)] * BASE for _ in range(BASE)]
    for y in range(BASE):
        t = y / (BASE - 1)
        # Slight ease so the horizon haze sits lower.
        tt = t ** 1.2
        bg = (lerp(top[0], bottom[0], tt), lerp(top[1], bottom[1], tt), lerp(top[2], bottom[2], tt))
        for x in range(BASE):
            px[y][x] = bg

    rand = rng(445566)
    horizon = int(BASE * 0.72)
    x = 0
    while x < BASE:
        w = int(40 + rand() * 110)
        h = int(BASE * (0.10 + rand() * 0.42))
        shade = 20 + int(rand() * 40)
        for bx in range(x, min(x + w, BASE)):
            for by in range(horizon - h, horizon):
                # Facade with faint window grid.
                win = 0
                if (bx - x) % 16 < 8 and (by - (horizon - h)) % 20 < 10:
                    win = 1
                c = shade + (26 if win else 0)
                px[by][bx] = (c, c + 6, c + 16)
        x += w + int(8 + rand() * 26)

    # Ground band.
    for y in range(horizon, BASE):
        for x in range(BASE):
            px[y][x] = (18, 26, 38)
    return px


def write_png(path, px, size):
    """Write an RGBA PNG for a size x size image, nearest-scaled.

    Tauri's icon loader requires RGBA (color type 6), so every pixel carries an
    opaque alpha byte even though the artwork is fully opaque.
    """
    step = BASE / size
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filter type 0
        sy = min(BASE - 1, int(y * step))
        row = px[sy]
        for x in range(size):
            sx = min(BASE - 1, int(x * step))
            r, g, b = row[sx]
            raw += bytes((r, g, b, 255))

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def write_ico(path, png_bytes, size):
    """Minimal single-image ICO wrapping a PNG payload."""
    header = struct.pack("<HHH", 0, 1, 1)
    w = 0 if size >= 256 else size
    entry = struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(png_bytes), 6 + 16)
    with open(path, "wb") as f:
        f.write(header + entry + png_bytes)


def main():
    os.makedirs(OUT, exist_ok=True)
    px = make_base()

    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
    }
    for name, size in sizes.items():
        write_png(os.path.join(OUT, name), px, size)

    # ICO (Windows): a 256px PNG payload.
    ico_png = open(os.path.join(OUT, "icon.png"), "rb").read()
    write_png("/tmp/urbix_ico_256.png", px, 256)
    write_ico(os.path.join(OUT, "icon.ico"), open("/tmp/urbix_ico_256.png", "rb").read(), 256)

    # ICNS (macOS): build an .iconset then run iconutil.
    if sys.platform == "darwin":
        iconset = "/tmp/urbix.iconset"
        subprocess.run(["rm", "-rf", iconset], check=False)
        os.makedirs(iconset, exist_ok=True)
        mapping = [
            (16, "icon_16x16.png"), (32, "icon_16x16@2x.png"),
            (32, "icon_32x32.png"), (64, "icon_32x32@2x.png"),
            (128, "icon_128x128.png"), (256, "icon_128x128@2x.png"),
            (256, "icon_256x256.png"), (512, "icon_256x256@2x.png"),
            (512, "icon_512x512.png"), (1024, "icon_512x512@2x.png"),
        ]
        for size, name in mapping:
            write_png(os.path.join(iconset, name), px, size)
        subprocess.run(
            ["iconutil", "-c", "icns", iconset, "-o", os.path.join(OUT, "icon.icns")],
            check=True,
        )
        subprocess.run(["rm", "-rf", iconset], check=False)
    else:
        # Non-mac fallback: an empty-ish placeholder is never used to bundle
        # here anyway; skip icns.
        print("note: not Darwin, skipping .icns generation")

    print("wrote icons to", OUT)


if __name__ == "__main__":
    main()
