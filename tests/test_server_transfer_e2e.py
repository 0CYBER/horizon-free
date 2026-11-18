"""Pure-Python regression for the VarInt decoder used by the hand-over relay.

``parse_varint`` reads handshake and login fields in the proxy that tools/server_transfer_e2e.py
puts in front of each engine. Minecraft VarInts are signed 32-bit values in at most five bytes,
so a sixth continuation byte is malformed and the fifth byte's top bit makes the value negative.
"""
import unittest

from tools.server_transfer_e2e import parse_varint


class ParseVarintTests(unittest.TestCase):
    def test_overlong_six_byte_varint_is_rejected(self):
        # A well-formed LEB128 encoding of 1, but a VarInt may not use six bytes.
        with self.assertRaises(ValueError):
            parse_varint(bytes.fromhex('808080808001'), 0)

    def test_five_byte_minus_one_is_a_signed_32_bit_value(self):
        self.assertEqual(parse_varint(bytes.fromhex('ffffffff0f'), 0), (-1, 5))

    def test_valid_encodings_still_decode_the_same(self):
        for raw, expected in (
            ('00', (0, 1)),
            ('01', (1, 1)),
            ('7f', (127, 1)),
            ('8001', (128, 2)),
            ('ac02', (300, 2)),
            ('ffffffff07', (2147483647, 5)),
            ('8080808008', (-2147483648, 5)),
        ):
            with self.subTest(raw=raw):
                self.assertEqual(parse_varint(bytes.fromhex(raw), 0), expected)


if __name__ == '__main__':
    unittest.main()
