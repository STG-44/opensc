#!/usr/bin/env python3
"""Reproduce the small code-native OpenSC icon without external dependencies."""
from pathlib import Path
import struct
import zlib


def png(size):
    shapes = [
        (15, 15, 34, 18), (15, 15, 18, 34), (66, 15, 85, 18), (82, 15, 85, 34),
        (15, 82, 34, 85), (15, 66, 18, 85), (66, 82, 85, 85), (82, 66, 85, 85),
        (27, 33, 47, 38), (27, 33, 32, 53), (27, 48, 47, 53),
        (42, 48, 47, 68), (27, 63, 47, 68),
        (54, 33, 74, 38), (54, 33, 59, 68), (54, 63, 74, 68),
    ]
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            px, py = (x + 0.5) * 100 / size, (y + 0.5) * 100 / size
            outside = (px < 9 and py < 9 and (px-9)**2+(py-9)**2 > 81) or (px > 91 and py < 9 and (px-91)**2+(py-9)**2 > 81) or (px < 9 and py > 91 and (px-9)**2+(py-91)**2 > 81) or (px > 91 and py > 91 and (px-91)**2+(py-91)**2 > 81)
            color = (0, 0, 0, 0) if outside else (17, 24, 21, 255)
            if any(a <= px <= c and b <= py <= d for a, b, c, d in shapes):
                color = (218, 177, 103, 255)
            row.extend(color)
        rows.append(bytes(row))

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)

    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>2I5B', size, size, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b''.join(rows), 9)) + chunk(b'IEND', b'')


if __name__ == '__main__':
    out = Path(__file__).resolve().parents[1] / 'assets'
    out.mkdir(exist_ok=True)
    (out / 'opensc.png').write_bytes(png(256))
    (out / 'opensc-1024.png').write_bytes(png(1024))
    sizes = [16, 32, 48, 64, 128, 256]
    images = [png(size) for size in sizes]
    offset = 6 + 16 * len(sizes)
    entries = []
    for size, data in zip(sizes, images):
        entries.append(struct.pack('<4B2H2I', size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    (out / 'opensc.ico').write_bytes(struct.pack('<3H', 0, 1, len(sizes)) + b''.join(entries) + b''.join(images))
