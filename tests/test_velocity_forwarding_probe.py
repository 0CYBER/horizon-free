import re
import subprocess
import sys
import unittest
from pathlib import Path

from tools import velocity_forwarding_probe as probe

ROOT = Path(__file__).resolve().parents[1]


class ProtocolVersionTests(unittest.TestCase):
    """The probe announces the protocol the game speaks, read from the one place it is written down."""

    def test_it_reads_the_pinned_protocol(self):
        version = (ROOT / "crates/horizon-core/src/version.rs").read_text()
        pinned = int(re.search(r"PROTOCOL_VERSION: i32 = (\d+);", version)[1])
        self.assertEqual(probe.protocol_version(), pinned)

    def test_it_fails_when_the_constant_is_gone(self):
        with self.assertRaises(SystemExit):
            probe.parse_protocol("pub const PROTOCOL_VERSION: i32 = ;")
    def test_the_default_is_the_pinned_protocol(self):
        """`--protocol` defaults to the game's, so a version bump reaches the probe."""
        result = subprocess.run([sys.executable, str(ROOT / "tools/velocity_forwarding_probe.py"),
                                 "join", "--help"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0)
        self.assertIn("--protocol", result.stdout)
        self.assertIn(str(probe.protocol_version()), result.stdout)


class NoProtocolLiteralTests(unittest.TestCase):
    def test_no_protocol_number_is_written_by_hand(self):
        """The probe names no protocol of its own; it reads the pinned one."""
        source = (ROOT / "tools/velocity_forwarding_probe.py").read_text()
        declaration = re.search(r'add_argument\("--protocol".*?\)\n', source, re.S).group(0)
        self.assertIn("default=protocol_version()", declaration)
        self.assertNotRegex(declaration, r"default=\d")


class VarintTests(unittest.TestCase):
    """VarInts are signed 32-bit ints: parse_varint must match Java/net_capture, not decode unsigned."""

    def test_sixth_byte_varints_are_refused(self):
        for raw in ("808080808001", "ffffffff8f00", "8080808080"):
            with self.subTest(raw=raw):
                with self.assertRaisesRegex(ValueError, "varint too long"):
                    probe.parse_varint(bytes.fromhex(raw))

    def test_negative_varints_decode_as_signed(self):
        for n in (-1, -2147483648, 0, 2147483647):
            with self.subTest(n=n):
                self.assertEqual(probe.parse_varint(probe.varint(n))[0], n)
        # The canonical -1 encoding must not come back as 4294967295.
        self.assertEqual(probe.parse_varint(bytes.fromhex("ffffffff0f"))[0], -1)

    def test_excess_fifth_group_bits_wrap_to_signed_32_bits(self):
        for raw, expected in (("ffffffff1f", -1), ("8080808010", 0),
                              ("8080808018", -2147483648)):
            with self.subTest(raw=raw):
                self.assertEqual(probe.parse_varint(bytes.fromhex(raw)), (expected, 5))


if __name__ == "__main__":
    unittest.main()
