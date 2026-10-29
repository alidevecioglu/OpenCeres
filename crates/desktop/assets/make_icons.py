#!/usr/bin/env python3
"""Makes the Windows and Linux icon files from icon-1024.png (Python stdlib
only; resizing uses macOS `sips`). Run again after changing the icon:

    python3 crates/desktop/assets/make_icons.py

Writes, next to this script:
  icon.ico         16/24/32/48/64/128 px as 32-bit BMP, 256 px as PNG
  icon.res         the same images as a compiled Windows resource (an icon
                   group with id 1), linked into the .exe by build.rs, so
                   no resource compiler or build dependency is needed
  icon-256.rgba.z  256 x 256 RGBA, zlib-compressed: the window icon on
                   Windows and Linux (inflated at start, no PNG decoder)
"""

import os
import struct
import subprocess
import sys
import tempfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = os.path.join(HERE, "icon-1024.png")
SIZES = [16, 24, 32, 48, 64, 128, 256]


def read_png(path):
    """Decodes an 8-bit, non-interlaced RGB or RGBA PNG to (w, h, rgba)."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path}: not a PNG")
    pos, idat, palette = 8, b"", None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            w, h, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        elif kind == b"PLTE":
            palette = body
    if depth != 8 or interlace or color not in (2, 6):
        sys.exit(f"{path}: need 8-bit RGB(A), non-interlaced (got type {color}, depth {depth})")
    bpp = 4 if color == 6 else 3
    raw = zlib.decompress(idat)
    stride = w * bpp
    out = bytearray()
    prev = bytearray(stride)
    i = 0
    for _ in range(h):
        f = raw[i]
        line = bytearray(raw[i + 1 : i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pred) & 255
        prev = line
        if bpp == 4:
            out += line
        else:
            for x in range(w):
                out += line[3 * x : 3 * x + 3] + b"\xff"
    return w, h, bytes(out)


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


def resized(size, tmp):
    out = os.path.join(tmp, f"{size}.png")
    subprocess.run(
        ["sips", "-z", str(size), str(size), SOURCE, "--out", out],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    return read_png(out)


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


def main():
    with tempfile.TemporaryDirectory() as tmp:
        images = []
        window = None
        for size in SIZES:
            w, h, rgba = resized(size, tmp)
            assert (w, h) == (size, size), (w, h)
            images.append((size, png(w, h, rgba) if size >= 256 else dib(w, h, rgba)))
            if size == 256:
                window = rgba
    with open(os.path.join(HERE, "icon.ico"), "wb") as f:
        f.write(ico(images))
    with open(os.path.join(HERE, "icon.res"), "wb") as f:
        f.write(res(images))
    with open(os.path.join(HERE, "icon-256.rgba.z"), "wb") as f:
        f.write(zlib.compress(window, 9))
    for name in ("icon.ico", "icon.res", "icon-256.rgba.z"):
        print(f"{name}: {os.path.getsize(os.path.join(HERE, name)):,} bytes")


if __name__ == "__main__":
    main()
