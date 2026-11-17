"""world_compare admits only real, dimension-correct comparison domains.

Both regressions drive the real CLI with independently built synthetic worlds in temp folders:
one full Anvil chunk carries one stone section and one 64-cell biome section, so a valid
comparison has observable coverage instead of an assumed one.
"""

import gzip
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TOOL = Path(__file__).resolve().parents[1] / "tools" / "world_compare.py"


def tag_string(text: str) -> bytes:
    data = text.encode("utf-8")
    return struct.pack(">H", len(data)) + data


def entry(tag_id: int, name: str, payload: bytes) -> bytes:
    return struct.pack(">B", tag_id) + tag_string(name) + payload


def compound(entries: list[bytes]) -> bytes:
    """A compound payload: its entries followed by the TAG_End that closes it."""
    return b"".join(entries) + b"\x00"


def list_of(tag_id: int, items: list[bytes]) -> bytes:
    """A TAG_List payload: element type, count, then the items' payloads (unnamed)."""
    return struct.pack(">B", tag_id) + struct.pack(">i", len(items)) + b"".join(items)


def level_dat() -> bytes:
    """A gzipped level.dat whose Data.spawn.pos is read by world_info."""
    spawn = entry(10, "spawn", compound([
        entry(11, "pos", struct.pack(">i", 3) + struct.pack(">iii", 0, 64, 0)),
    ]))
    return gzip.compress(entry(10, "", compound([
        entry(10, "Data", compound([spawn])),
    ])))


def world_gen_settings(seed: int = 42) -> bytes:
    """A gzipped world_gen_settings.dat whose data.seed is read by world_info."""
    return gzip.compress(entry(10, "", compound([
        entry(10, "data", compound([
            entry(4, "seed", struct.pack(">q", seed)),
        ])),
    ])))


def full_chunk(biome: str = "minecraft:plains") -> bytes:
    """A full chunk with one stone section (4096 blocks) and one 64-cell biome section."""
    section = compound([
        entry(1, "Y", struct.pack(">b", 0)),
        entry(10, "block_states", compound([
            entry(9, "palette", list_of(8, [tag_string("minecraft:stone")])),
        ])),
        entry(10, "biomes", compound([
            entry(9, "palette", list_of(8, [tag_string(biome)])),
        ])),
    ])
    return entry(10, "", compound([
        entry(3, "xPos", struct.pack(">i", 0)),
        entry(3, "zPos", struct.pack(">i", 0)),
        entry(8, "Status", tag_string("minecraft:full")),
        entry(9, "sections", list_of(10, [section])),
    ]))


def structure_only_chunk() -> bytes:
    """A chunk with a structure start but no terrain sections and no biomes yet."""
    start = entry(10, "minecraft:igloo", compound([
        entry(3, "ChunkX", struct.pack(">i", 0)),
        entry(3, "ChunkZ", struct.pack(">i", 0)),
        entry(8, "id", tag_string("minecraft:igloo")),
    ]))
    return entry(10, "", compound([
        entry(3, "xPos", struct.pack(">i", 0)),
        entry(3, "zPos", struct.pack(">i", 0)),
        entry(8, "Status", tag_string("minecraft:structure_starts")),
        entry(10, "structures", compound([entry(10, "starts", compound([start]))])),
    ]))


def region_file(chunks: dict[tuple[int, int], bytes]) -> bytes:
    """An Anvil region file with uncompressed (codec 3) chunk records."""
    header = bytearray(4096)
    blocks = bytearray()
    for index, ((cx, cz), raw) in enumerate(sorted(chunks.items())):
        sector = 2 + index
        offset = (cx + cz * 32) * 4
        header[offset:offset + 3] = sector.to_bytes(3, "big")
        blocks += struct.pack(">I", len(raw) + 1) + b"\x03" + raw
        while len(blocks) % 4096:
            blocks += b"\x00"
    return bytes(header) + b"\x00" * 4096 + bytes(blocks)


def make_world(root: Path, name: str, *, chunk: bytes | None = None,
               dimension: str | None = None) -> Path:
    """A scratch world with valid metadata and either root or dimension-scoped region data."""
    world = root / name
    world.mkdir(parents=True)
    (world / "level.dat").write_bytes(level_dat())
    settings = world / "data" / "minecraft"
    settings.mkdir(parents=True)
    (settings / "world_gen_settings.dat").write_bytes(world_gen_settings())
    folder = world / "region"
    if dimension is not None:
        folder = world / "dimensions" / "minecraft" / dimension / "region"
    folder.mkdir(parents=True)
    if chunk is not None:
        (folder / "r.0.0.mca").write_bytes(region_file({(0, 0): chunk}))
    return world


def run_tool(root: Path, *args) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, str(TOOL), *map(str, args)],
                          cwd=root, text=True, capture_output=True)


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)

    def test_empty_and_structure_only_domains_are_rejected(self):
        a = make_world(self.root, "emptyA")
        b = make_world(self.root, "emptyB")
        result = run_tool(self.root, a, b)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("empty comparison domain", result.stdout)
        self.assertIn("0 of 0 cells differ", result.stdout)
        self.assertIn("RESULT: DIFFERS", result.stdout)

        a = make_world(self.root, "structureA", chunk=structure_only_chunk())
        b = make_world(self.root, "structureB", chunk=structure_only_chunk())
        result = run_tool(self.root, a, b)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("in both 1", result.stdout, "the structure start was actually compared")
        self.assertIn("empty comparison domain", result.stdout)

        # Control: the same CLI and root region layout with real coverage still matches.
        a = make_world(self.root, "fullA", chunk=full_chunk())
        b = make_world(self.root, "fullB", chunk=full_chunk())
        ok = run_tool(self.root, a, b)
        self.assertEqual(ok.returncode, 0, ok.stdout + ok.stderr)
        self.assertIn("1 chunks with biomes in both, 1 identical, 0 of 64 cells differ", ok.stdout)
        self.assertIn("RESULT: matches", ok.stdout)

    def test_requested_dimension_is_resolved_without_overworld_fallback(self):
        # Populated overworld only: asking for the Nether must not compare it.
        a = make_world(self.root, "overworldA", chunk=full_chunk())
        b = make_world(self.root, "overworldB", chunk=full_chunk())
        result = run_tool(self.root, a, b, "--dimension", "the_nether")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("dimension 'the_nether' has no region folder", result.stdout)
        self.assertIn("RESULT: DIFFERS", result.stdout)

        # Control: real Nether data is resolved and read, not substituted.
        a = make_world(self.root, "netherA", dimension="the_nether", chunk=full_chunk())
        b = make_world(self.root, "netherB", dimension="the_nether", chunk=full_chunk())
        ok = run_tool(self.root, a, b, "--dimension", "the_nether")
        self.assertEqual(ok.returncode, 0, ok.stdout + ok.stderr)
        self.assertIn("1 chunks with biomes in both, 1 identical, 0 of 64 cells differ", ok.stdout)
        self.assertIn("RESULT: matches", ok.stdout)

        a = make_world(self.root, "netherDiffA", dimension="the_nether",
                       chunk=full_chunk("minecraft:plains"))
        b = make_world(self.root, "netherDiffB", dimension="the_nether",
                       chunk=full_chunk("minecraft:desert"))
        bad = run_tool(self.root, a, b, "--dimension", "the_nether")
        self.assertEqual(bad.returncode, 1, bad.stdout + bad.stderr)
        self.assertIn("64 of 64 cells differ", bad.stdout, "the Nether chunks were compared")


if __name__ == "__main__":
    unittest.main()
