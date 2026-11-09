#!/usr/bin/env python3
"""Draws the OpenCeres mark and writes every icon file from it. Stdlib only,
so it runs anywhere; from the repository root:

    docker run --rm -v "$PWD":/w -w /w python:3.12-slim \\
        python3 crates/desktop/assets/make_icons.py

The mark is what the app does: one object caught on three frames (three amber
dots on a rising line), the last position ringed as the measured one, on a
navy tile. `paint_logo` in src/app.rs draws the same geometry and has to be
changed along with it.

Each size is drawn, never resampled, so small icons stay sharp and follow a
ladder: below 32 px the dots only (larger, on whole pixels), from 32 px the
ring, from 128 px two faint stars.

Writes, next to this script:
  icon-1024.png    full-bleed tile; also shipped in the Linux archive
  icon.icns        macOS, 16 to 1024 px, tile inside Apple's safe area
  icon.ico         16/24/32/48/64/128 px as 32-bit BMP, 256 px as PNG
  icon.res         the same images as a compiled Windows resource (an icon
                   group with id 1), linked into the .exe by build.rs, so
                   no resource compiler or build dependency is needed
  icon-256.rgba.z  256 x 256 RGBA, zlib-compressed: the window icon on
                   Windows and Linux (inflated at start, no PNG decoder)
and in docs/assets the website's favicons and icons.
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
DOCS = os.path.join(HERE, "..", "..", "..", "docs", "assets")
SIZES = [16, 24, 32, 48, 64, 128, 256]

NAVY = (0x0d, 0x16, 0x36)
AMBER = (0xf2, 0xb3, 0x3d)
STAR = (0xe8, 0xec, 0xf7)

# Geometry on a 32-unit tile. Spacing is even because the frames are: an
# object in steady motion moves the same distance between exposures.
CORNER = 7.2  # 22.5 % of the tile, the macOS icon corner
DOTS = [(7.0, 25.0), (14.0, 18.0), (21.0, 11.0)]
DOT = 2.25
RING = 5.5  # centreline radius, around the last dot
RING_WIDTH = 1.5
STARS = [(9.5, 8.5, 0.6), (25.5, 23.5, 0.45)]
STAR_OPACITY = 0.55
# Below 32 px there is no ring to leave room for: bigger dots, centred, on
# positions that land on whole pixels at 16 and 24 px.
SMALL_DOTS = [(8.0, 24.0), (16.0, 16.0), (24.0, 8.0)]
SMALL_DOT = 3.0

# Apple's macOS template: an 824 px tile centred on the 1024 px canvas.
MAC_TILE = 824 / 1024


def coverage(distance):
    """Pixel coverage from the signed distance to a shape's edge."""
    return max(0.0, min(1.0, 0.5 - distance))


def stamp(buf, size, cx, cy, outer, inner, color, opacity=1.0):
    """Blends a disc (inner = 0) or ring onto the tile."""
    x0, x1 = max(0, int(cx - outer - 1)), min(size, int(cx + outer + 2))
    y0, y1 = max(0, int(cy - outer - 1)), min(size, int(cy + outer + 2))
    for y in range(y0, y1):
        for x in range(x0, x1):
            r = math.hypot(x + 0.5 - cx, y + 0.5 - cy)
            d = max(r - outer, inner - r) if inner else r - outer
            a = coverage(d) * opacity
            if a:
                i = 4 * (y * size + x)
                for k in range(3):
                    buf[i + k] = round(buf[i + k] + (color[k] - buf[i + k]) * a)


def mark(size, tile):
    """The mark as RGBA on a `size` px canvas, with the tile `tile` px wide.
    Outside the tile the colour stays navy at zero alpha, so that scaling by
    the OS cannot pull a dark fringe in from black."""
    u = tile / 32
    origin = (size - tile) / 2
    corner = CORNER * u
    half = tile / 2
    buf = bytearray(bytes(NAVY) + b"\0") * (size * size)
    for y in range(size):
        for x in range(size):
            qx = abs(x + 0.5 - size / 2) - (half - corner)
            qy = abs(y + 0.5 - size / 2) - (half - corner)
            outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
            d = outside + min(max(qx, qy), 0.0) - corner
            buf[4 * (y * size + x) + 3] = round(coverage(d) * 255)
    at = lambda v: origin + v * u
    if size < 32:
        dots, radius = SMALL_DOTS, SMALL_DOT
    else:
        dots, radius = DOTS, DOT
    if size >= 128:
        for x, y, r in STARS:
            stamp(buf, size, at(x), at(y), r * u, 0, STAR, STAR_OPACITY)
    for x, y in dots:
        stamp(buf, size, at(x), at(y), radius * u, 0, AMBER)
    if size >= 32:
        x, y = DOTS[-1]
        w = RING_WIDTH * u / 2
        stamp(buf, size, at(x), at(y), RING * u + w, RING * u - w, AMBER)
    return bytes(buf)


def full(size):
    return mark(size, size)


def mac(size):
    return mark(size, size * MAC_TILE)


def png(w, h, rgba):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    rows = b"".join(b"\x00" + rgba[y * w * 4 : (y + 1) * w * 4] for y in range(h))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def dib(w, h, rgba):
    """Icon image as a DIB: BITMAPINFOHEADER (height doubled), BGRA rows
    bottom-up, then the 1-bit AND mask (all zero: alpha decides)."""
    header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    pixels = bytearray()
    for y in reversed(range(h)):
        row = rgba[y * w * 4 : (y + 1) * w * 4]
        for x in range(w):
            r, g, b, a = row[4 * x : 4 * x + 4]
            pixels += bytes((b, g, r, a))
    mask_stride = ((w + 31) // 32) * 4
    return header + bytes(pixels) + bytes(mask_stride * h)


def ico(images):
    head = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for size, blob in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        blobs += blob
        offset += len(blob)
    return head + entries + blobs


def res(images):
    """A .res file: an empty first entry, one RT_ICON per image (ids 1..n)
    and an RT_GROUP_ICON (id 1) listing them, language en-US."""

    def entry(kind, name, flags, body):
        header = struct.pack("<HHHH", 0xFFFF, kind, 0xFFFF, name)
        header += struct.pack("<IHHII", 0, flags, 0x0409, 0, 0)
        out = struct.pack("<II", len(body), 8 + len(header)) + header + body
        return out + b"\0" * (-len(out) % 4)

    out = struct.pack("<II", 0, 32) + struct.pack("<HHHH", 0xFFFF, 0, 0xFFFF, 0) + bytes(16)
    group = struct.pack("<HHH", 0, 1, len(images))
    for n, (size, blob) in enumerate(images, start=1):
        out += entry(3, n, 0x1010, blob)  # RT_ICON, MOVEABLE | DISCARDABLE
        dim = 0 if size >= 256 else size
        group += struct.pack("<BBBBHHIH", dim, dim, 0, 0, 1, 32, len(blob), n)
    out += entry(14, 1, 0x1030, group)  # RT_GROUP_ICON, + PURE
    return out


def packbits(channel):
    """The icns run-length code: n < 128 copies n + 1 bytes, n >= 128 repeats
    the next byte n - 125 times."""
    out, i = bytearray(), 0
    while i < len(channel):
        run = 1
        while i + run < len(channel) and run < 130 and channel[i + run] == channel[i]:
            run += 1
        if run >= 3:
            out += bytes((run + 125, channel[i]))
            i += run
            continue
        start = i
        while i < len(channel) and i - start < 128:
            if i + 2 < len(channel) and channel[i] == channel[i + 1] == channel[i + 2]:
                break
            i += 1
        out += bytes((i - start - 1,)) + bytes(channel[start:i])
    return bytes(out)


def argb(rgba):
    """The ic04/ic05 payload: 'ARGB', then each channel run-length coded."""
    order = (3, 0, 1, 2)
    return b"ARGB" + b"".join(packbits(rgba[k::4]) for k in order)


def icns():
    """PNG for every entry except the two smallest, which macOS expects as
    run-length ARGB."""
    entries = [
        (b"ic04", argb(mac(16))),
        (b"ic05", argb(mac(32))),
        (b"ic11", png(32, 32, mac(32))),
        (b"ic12", png(64, 64, mac(64))),
        (b"ic07", png(128, 128, mac(128))),
    ]
    for kind, size in [(b"ic13", 256), (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024)]:
        entries.append((kind, png(size, size, mac(size))))
    body = b"".join(kind + struct.pack(">I", len(blob) + 8) + blob for kind, blob in entries)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def main():
    images = []
    for size in SIZES:
        rgba = full(size)
        images.append((size, png(size, size, rgba) if size >= 256 else dib(size, size, rgba)))
        if size == 256:
            window = rgba
    outputs = {
        "icon-1024.png": png(1024, 1024, full(1024)),
        "icon.icns": icns(),
        "icon.ico": ico(images),
        "icon.res": res(images),
        "icon-256.rgba.z": zlib.compress(window, 9),
    }
    for name, data in outputs.items():
        with open(os.path.join(HERE, name), "wb") as f:
            f.write(data)
        print(f"{name}: {len(data):,} bytes")
    for name, size in [("favicon-16.png", 16), ("favicon.png", 64), ("icon-256.png", 256), ("icon-512.png", 512)]:
        data = png(size, size, full(size))
        with open(os.path.join(DOCS, name), "wb") as f:
            f.write(data)
        print(f"docs/assets/{name}: {len(data):,} bytes")


if __name__ == "__main__":
    main()
