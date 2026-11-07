#!/usr/bin/env python3
"""Generate the project's small, reproducible material tiles using the stdlib."""
import argparse
import binascii
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / "apps/desktop/assets/materials"
SIDE = 128


def png(pixels):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", binascii.crc32(body))

    rows = b"".join(b"\0" + pixels[y * SIDE * 4:(y + 1) * SIDE * 4] for y in range(SIDE))
    header = struct.pack(">IIBBBBB", SIDE, SIDE, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")


def generate():
    # Xorshift32 and a fixed seed avoid Python-version-dependent random streams.
    state = 0x4E43544B
    noise = bytearray()
    for _ in range(SIDE * SIDE):
        state ^= (state << 13) & 0xFFFFFFFF
        state ^= state >> 17
        state ^= (state << 5) & 0xFFFFFFFF
        state &= 0xFFFFFFFF
        value = 255 if state & 1 else 0
        # 6/255 is 2.35% opacity, leaving contrast headroom even when a
        # light glass tint is composited over a black background.
        noise.extend((value, value, value, 6))

    def grid(color):
        data = bytearray()
        for y in range(SIDE):
            for x in range(SIDE):
                # A tiny rounded, 4.7%-opacity dot per 16 logical pixels.
                dot = abs(x % 16 - 8) + abs(y % 16 - 8) <= 1
                data.extend((*color, 12 if dot else 0))
        return data

    return {
        "grain.png": png(noise),
        "grid-light.png": png(grid((53, 111, 193))),
        "grid-dark.png": png(grid((159, 199, 255))),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify the committed bytes")
    args = parser.parse_args()
    if not args.check:
        DEST.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        path = DEST / name
        if args.check:
            if not path.is_file() or path.read_bytes() != data:
                raise SystemExit(f"{path} is missing or differs; regenerate material tiles")
        else:
            path.write_bytes(data)


if __name__ == "__main__":
    main()
