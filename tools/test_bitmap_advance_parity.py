#!/usr/bin/env python3
"""Compare bitmap font glyph advances with vanilla's real BitmapProvider.

Each case is a one-glyph bitmap provider over a synthetic sheet whose first `width` columns are opaque, for
positive and negative `height`s (a negative scale makes `(int) (0.5 + width * scale)` differ from flooring).
"""
import itertools
import json
import shutil
import struct
import zlib

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab
    import oracle_harness as harness

HEIGHTS = (8, 16, 7, -8, -16, -3)
CELLS = (8, 16)


def png(size, width):
    rows = b''.join(b'\0' + (b'\xff\xff\xff\xff' * width + bytes(4 * (size - width))) for _ in range(size))

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def make_resources(build):
    root = build / 'resources'
    shutil.rmtree(root, ignore_errors=True)
    textures = root / 'assets/minecraft/textures/font'
    textures.mkdir(parents=True)
    providers = []
    for index, (cell, height, width) in enumerate(
            (cell, height, width) for cell in CELLS for height in HEIGHTS for width in range(cell + 1)):
        name = f'cell{cell}_{width}'
        (textures / (name + '.png')).write_bytes(png(cell, width))
        providers.append(dict(type='bitmap', file=f'minecraft:font/{name}.png', height=height,
                              ascent=min(7, height), chars=[chr(0xE000 + index)]))
    fonts = root / 'assets/minecraft/font'
    fonts.mkdir(parents=True)
    (fonts / 'default.json').write_text(json.dumps(dict(providers=providers)))
    return root, len(providers)


if __name__ == '__main__':
    harness.differential(
        'bitmap-advance-parity', 'VanillaBitmapAdvanceOracle', 'bitmap_advance_fixture', make_resources,
        'BitmapProvider.Definition.load advances for 8 and 16 pixel cells, every opaque width from 0 to the '
        'cell width, and heights 8, 16, 7, -8, -16 and -3. Excludes glyph bounds and atlas packing.',
        sources=[lab.ROOT / 'crates/horizon-client-render/src/font.rs'])
