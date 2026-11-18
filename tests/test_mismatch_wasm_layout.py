"""tools/ci/mismatch_wasm_layout.py: uint must reject malformed u32 LEB128."""
import unittest

from tools.ci import mismatch_wasm_layout as mm


class Uint(unittest.TestCase):
    def test_valid_encodings_decode_identically(self):
        cases = [
            (b"\x00", 0),
            (b"\x7f", 127),
            (b"\x80\x01", 128),
            (b"\xff\x7f", 16383),
            (b"\x80\x80\x01", 16384),
            (b"\x80\x80\x80\x01", 2097152),
            (b"\xff\xff\xff\xff\x07", 2147483647),
            (b"\xff\xff\xff\xff\x0f", 4294967295),
        ]
        for data, expected in cases:
            with self.subTest(data=data.hex()):
                self.assertEqual(mm.uint(data, 0), (expected, len(data)))

    def test_offset_is_advanced_past_the_integer(self):
        data = b"\xaa\x80\x01\xbb"
        self.assertEqual(mm.uint(data, 1), (128, 3))

    def test_overlong_encodings_are_rejected(self):
        for data in (
            bytes.fromhex("80808080808000"),  # the repro's 7-byte encoding of zero
            bytes.fromhex("808080808000"),    # six bytes
            b"\x80\x80\x80\x80\x80",          # five continuation bytes, no terminator
        ):
            with self.subTest(data=data.hex()):
                with self.assertRaises(ValueError) as raised:
                    mm.uint(data, 0)
                self.assertEqual(str(raised.exception), "overlong LEB128")

    def test_truncated_encodings_are_rejected(self):
        for data in (b"\x80", b"\x80\x80", b"\x80\x80\x80\x80"):
            with self.subTest(data=data.hex()):
                with self.assertRaises(ValueError) as raised:
                    mm.uint(data, 0)
                self.assertEqual(str(raised.exception), "truncated LEB128")

    def test_fifth_byte_must_fit_in_u32(self):
        with self.assertRaises(ValueError) as raised:
            mm.uint(b"\xff\xff\xff\xff\x7f", 0)
        self.assertEqual(str(raised.exception), "LEB128 value out of range for u32")


if __name__ == "__main__":
    unittest.main()
