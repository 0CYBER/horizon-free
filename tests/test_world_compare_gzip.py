"""Chunk-read regression: world_compare.read_chunks must admit every Anvil codec it supports.

The region/NBT bytes below are built independently in this file; the expected values are the
compounds we wrote, not output captured from the tool. Every case calls the real read_chunks on a
real region file and compares the whole returned dict (chunk set, coordinates and contents).
"""

import gzip
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools import anvil_dump, world_compare  # noqa: E402


def utf(text):
    raw = text.encode("utf-8")
    return struct.pack(">H", len(raw)) + raw


def tag(kind, name, payload):
    return struct.pack(">B", kind) + utf(name) + payload


def chunk(cx, cz, marker):
    """A named TAG_Compound with absolute chunk coordinates and a unique content marker."""
    body = tag(10, "", b"".join([
        tag(3, "xPos", struct.pack(">i", cx)),
        tag(3, "zPos", struct.pack(">i", cz)),
        tag(8, "Status", utf("minecraft:full")),
        tag(8, "Marker", utf(marker)),
    ]) + b"\x00")
    return body, {"xPos": cx, "zPos": cz, "Status": "minecraft:full", "Marker": marker}


def stored(codec, body):
    if codec == 1:
        return gzip.compress(body)
    if codec == 2:
        return zlib.compress(body)
    return body


def region(entries):
    """An Anvil region: {(local x, local z): (compression byte, NBT body)}, one sector each."""
    header = bytearray(4096)
    blocks = bytearray()
    for index, ((lx, lz), (codec, body)) in enumerate(sorted(entries.items())):
        at = (lx + lz * 32) * 4
        header[at:at + 3] = (2 + index).to_bytes(3, "big")
        header[at + 3] = 1
        payload = stored(codec, body)
        blocks += struct.pack(">I", len(payload) + 1) + bytes([codec]) + payload
        blocks += b"\x00" * ((-len(blocks)) % 4096)
    return bytes(header) + b"\x00" * 4096 + bytes(blocks)


class ChunkReadCodecMatrix(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.base = Path(scratch.name)

    def write(self, case, entries, name="r.0.0.mca"):
        folder = self.base / case
        (folder / "region").mkdir(parents=True)
        path = folder / "region" / name
        path.write_bytes(region(entries))
        return folder / "region", path

    def test_codec_matrix(self):
        for label, codec in (("gzip", 1), ("zlib", 2), ("uncompressed", 3)):
            with self.subTest(codec=label):
                body, expected = chunk(0, 0, label)
                folder, path = self.write(label, {(0, 0): (codec, body)})
                self.assertEqual(world_compare.read_chunks(str(folder)), {(0, 0): expected})
                # The independent Anvil reader sees the same chunk and the same NBT.
                self.assertEqual(anvil_dump.region_chunks(path), {(0, 0): expected})

        with self.subTest(codec="mixed"):
            entries = {}
            expected = {}
            for (lx, lz), codec, marker in (((0, 0), 1, "gzip"), ((1, 0), 2, "zlib"), ((5, 3), 3, "plain")):
                cx, cz = lx + 32, lz + 64
                body, chunk_value = chunk(cx, cz, marker)
                entries[(lx, lz)] = (codec, body)
                expected[(cx, cz)] = chunk_value
            folder, path = self.write("mixed", entries, "r.1.2.mca")
            self.assertEqual(world_compare.read_chunks(str(folder)), expected)
            self.assertEqual(set(anvil_dump.region_chunks(path)), {(0, 0), (1, 0), (5, 3)})

        with self.subTest(codec="unsupported"):
            body, _ = chunk(2, 1, "ignored")
            folder, _ = self.write("unsupported", {(2, 1): (4, body)})
            with self.assertRaises(ValueError) as raised:
                world_compare.read_chunks(str(folder))
            message = str(raised.exception)
            self.assertIn("r.0.0.mca", message)
            self.assertIn("2, 1", message)
            self.assertIn("compression type 4", message)

        with self.subTest(codec="unsupported beside a supported chunk"):
            gzip_body, _ = chunk(2, 1, "valid")
            other_body, _ = chunk(3, 1, "ignored")
            folder, _ = self.write("partial", {
                (2, 1): (1, gzip_body),
                (3, 1): (9, other_body),
            })
            with self.assertRaises(ValueError):
                world_compare.read_chunks(str(folder))


if __name__ == "__main__":
    unittest.main()
