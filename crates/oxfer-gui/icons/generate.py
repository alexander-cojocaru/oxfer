#!/usr/bin/env python3
"""Generate the oxfer app icons (PNG, ICO, ICNS) using only the standard library.

Run from anywhere: python3 crates/oxfer-gui/icons/generate.py
The design: a rounded square with a blue/indigo gradient and two white arrows
(upload / download).
"""

import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SS = 4  # supersampling factor per axis


def rounded_rect(x, y, r):
    # unit square inset by 0.06, corner radius r
    lo, hi = 0.06, 0.94
    if x < lo or x > hi or y < lo or y > hi:
        return False
    cx = min(max(x, lo + r), hi - r)
    cy = min(max(y, lo + r), hi - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def arrow_right(x, y, cy):
    # shaft
    if 0.24 <= x <= 0.62 and abs(y - cy) <= 0.045:
        return True
    # head: triangle with tip at (0.78, cy)
    if 0.56 <= x <= 0.78:
        half = (0.78 - x) * 0.75
        return abs(y - cy) <= half
    return False


def arrow_left(x, y, cy):
    return arrow_right(1.0 - x, y, cy)


def pixel(x, y):
    if not rounded_rect(x, y, 0.2):
        return (0, 0, 0, 0)
    if arrow_right(x, y, 0.36) or arrow_left(x, y, 0.64):
        return (255, 255, 255, 255)
    t = (x + y) / 2
    a = (79, 70, 229)  # indigo
    b = (14, 165, 233)  # sky
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3)) + (255,)


def render(size):
    rows = []
    n = size * SS
    for py in range(size):
        row = bytearray()
        for px in range(size):
            acc = [0, 0, 0, 0]
            for sy in range(SS):
                for sx in range(SS):
                    x = (px * SS + sx + 0.5) / n
                    y = (py * SS + sy + 0.5) / n
                    r, g, b, a = pixel(x, y)
                    acc[0] += r * a
                    acc[1] += g * a
                    acc[2] += b * a
                    acc[3] += a
            alpha = acc[3] / (SS * SS)
            if acc[3]:
                row += bytes(round(acc[i] / acc[3]) for i in range(3))
            else:
                row += b"\0\0\0"
            row.append(round(alpha))
        rows.append(bytes(row))
    return rows


def png(size):
    rows = render(size)
    raw = b"".join(b"\0" + r for r in rows)

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def ico(images):
    # images: list of (size, png bytes); PNG-compressed ICO entries
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, data = b"", b""
    for size, blob in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
        data += blob
    return header + entries + data


def icns(images):
    # images: list of (OSType, png bytes)
    body = b"".join(t + struct.pack(">I", len(b) + 8) + b for t, b in images)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def main():
    cache = {}

    def get(size):
        if size not in cache:
            cache[size] = png(size)
        return cache[size]

    files = {
        "32x32.png": get(32),
        "128x128.png": get(128),
        "128x128@2x.png": get(256),
        "icon.png": get(512),
        "icon.ico": ico([(s, get(s)) for s in (16, 24, 32, 48, 64, 256)]),
        "icon.icns": icns(
            [
                (b"icp4", get(16)),
                (b"icp5", get(32)),
                (b"icp6", get(64)),
                (b"ic07", get(128)),
                (b"ic08", get(256)),
                (b"ic09", get(512)),
            ]
        ),
    }
    for name, blob in files.items():
        with open(os.path.join(HERE, name), "wb") as f:
            f.write(blob)
        print(f"wrote {name} ({len(blob)} bytes)")


if __name__ == "__main__":
    main()
