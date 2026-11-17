"""NBT readers must decode Java modified UTF-8 in both keys and values."""

import json
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools import anvil_dump, modified_utf8, world_compare  # noqa: E402

ANVIL = ROOT / "tools" / "anvil_dump.py"
HELPER = ROOT / "tools" / "modified_utf8.py"

# Independent literal vectors. The bytes follow DataOutputStream.writeUTF, and
# the expected strings are written out by hand, never computed by the decoder.
# The rocket key/value pair is the exact JDK 25 oracle from issue #17289.
VECTORS = [
    ("48656c6c6f", "Hello"),                       # plain ASCII control
    ("636166c3a9", "café"),                         # two-byte BMP
    ("e282ac", "\u20ac"),                           # three-byte BMP
    ("61c08062", "a\x00b"),                         # encoded NUL C0 80
    ("610062", "a\x00b"),                           # raw 0x00, accepted by readUTF
    ("c181", "A"),                                  # overlong two-byte form
    ("e080af", "/"),                                # overlong three-byte form
    ("6b6579eda0bdedba80", "key\U0001f680"),       # UTF-16 surrogate pair
    ("eda0bd", "\ud83d"),                           # isolated high surrogate
    ("edba80", "\ude80"),                           # isolated low surrogate
    ("edba80eda0bd", "\ude80\ud83d"),               # low then high: two isolated units
]

ISSUE_VECTOR = "0a00000800096b6579eda0bdedba80000d76616c7565c080eda0bdedba8000"
ISSUE_EXPECTED = {"key\U0001f680": "value\x00\U0001f680"}

BAD = [
    ("c3", "truncated two-byte form"),
    ("eda0", "truncated three-byte form"),
    ("c328", "illegal two-byte continuation"),
    ("eda028", "illegal three-byte continuation"),
    ("80", "bare continuation byte"),
    ("f09f9a80", "four-byte UTF-8 is not modified UTF-8"),
]


def counted(payload):
    return struct.pack(">H", len(payload)) + payload


def string_entry(payload):
    """A TAG_String whose key and value are both the given MUTF-8 payload."""
    return b"\x08" + counted(payload) + counted(payload)


def named_root(entries):
    """A named TAG_Compound root, the shape a level.dat has."""
    return b"\x0a" + counted(b"") + b"".join(entries) + b"\x00"


def cli(*args, cwd=None):
    return subprocess.run([sys.executable, *map(str, args)], cwd=cwd,
                          text=True, capture_output=True)


class ModifiedUtf8Matrix(unittest.TestCase):
    def test_modified_utf8_nbt_matrix(self):
        # 1. The literal JDK 25 oracle, through both real readers.
        raw = bytes.fromhex(ISSUE_VECTOR)
        self.assertEqual(anvil_dump.read_named(raw), ISSUE_EXPECTED)
        self.assertEqual(world_compare.parse_nbt(raw), ISSUE_EXPECTED)

        # 2. The vector matrix, through both real readers, as NBT keys and values.
        for hexbytes, expected in VECTORS:
            payload = bytes.fromhex(hexbytes)
            with self.subTest(vector=hexbytes):
                nbt = named_root([string_entry(payload)])
                self.assertEqual(anvil_dump.read_named(nbt), {expected: expected})
                self.assertEqual(world_compare.parse_nbt(nbt), {expected: expected})

        # 3. CLI controls: a real .dat dump and the helper's own CLI.
        with tempfile.TemporaryDirectory() as folder:
            (Path(folder) / "level.dat").write_bytes(bytes.fromhex(ISSUE_VECTOR))
            dumped = cli(ANVIL, "level.dat", cwd=folder)
            self.assertEqual(dumped.returncode, 0, dumped.stderr)
            self.assertEqual(json.loads(dumped.stdout), ISSUE_EXPECTED)

            for hexbytes, expected in VECTORS:
                with self.subTest(cli=hexbytes):
                    result = cli(HELPER, hexbytes)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(json.loads(result.stdout), expected)

            streamed = cli(HELPER, "--stream", "0009" + VECTORS[7][0])
            self.assertEqual(streamed.returncode, 0, streamed.stderr)
            self.assertEqual(json.loads(streamed.stdout), VECTORS[7][1])

        # 4. Malformed sequences fail in the helper and in both readers.
        for hexbytes, reason in BAD:
            bad = bytes.fromhex(hexbytes)
            with self.subTest(bad=hexbytes, reason=reason):
                with self.assertRaises(ValueError):
                    modified_utf8.decode_mutf8(bad)
                nbt = named_root([string_entry(bad)])
                with self.assertRaises(ValueError):
                    anvil_dump.read_named(nbt)
                with self.assertRaises(ValueError):
                    world_compare.parse_nbt(nbt)


if __name__ == "__main__":
    unittest.main()
