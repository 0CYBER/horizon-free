"""Pure-Python tests for tools/datafix_worlds.py's decoder and configuration loop.

No server, network, JVM or Mojang data is needed: packet buffers and a
byte-recording socket stand-in exercise the actual Bot methods.
"""
import os
import tempfile
from types import SimpleNamespace
import unittest

# datafix_worlds resolves its writable root at import time; keep it off any real
# player data by pointing it at a scratch directory before importing the module.
os.environ.setdefault('HORIZON_DATAFIX_ROOT', tempfile.mkdtemp(prefix='horizon-u02-'))

from tools import datafix_worlds  # noqa: E402


def read_varint(raw_hex):
    """Decode a hex byte string with a Bot that has no socket, only a buffer."""
    bot = datafix_worlds.Bot.__new__(datafix_worlds.Bot)
    bot.buffer = bytearray(bytes.fromhex(raw_hex))
    return bot.read_varint()


class BotReadVarint(unittest.TestCase):
    def test_canonical_encodings_decode_identically(self):
        cases = {
            '00': 0,
            '01': 1,
            '7f': 127,
            '8001': 128,
            'ffffffff07': 2147483647,
        }
        for raw, expected in cases.items():
            with self.subTest(raw=raw):
                self.assertEqual(read_varint(raw), expected)

    def test_overlong_varint_raises(self):
        # Five 0x80 continuation bytes then 0x01: a sixth group must be refused,
        # as vanilla VarInt.read does, instead of decoding to 2**35.
        with self.assertRaisesRegex(ValueError, 'VarInt too big'):
            read_varint('808080808001')


class BotConfiguration(unittest.TestCase):
    def test_unrelated_uuid_payload_does_not_abort_configuration(self):
        bot = datafix_worlds.Bot.__new__(datafix_worlds.Bot)
        bot.protocol = 766
        bot.threshold = -1
        # Resource-pack UUID, empty URL/hash, and false required/prompt flags.
        # Its UUID prefix is not a VarInt and must not abort known-packs detection.
        resource_pack = b'\x80' * 5 + b'\x00' * 11 + b'\x00' * 4
        bot.buffer = (b'\x15\x09' + resource_pack
                      + b'\x18\x0e\x01\x09minecraft\x04core\x061.20.5'
                      + b'\x01\x03')
        sent = []
        bot.sock = SimpleNamespace(sendall=sent.append)

        bot.configure()

        # Actual wire frames: no response to the unrelated payload, then an empty
        # known-packs selection and the finish-configuration acknowledgment.
        self.assertEqual(sent, [b'\x02\x07\x00', b'\x01\x03'])


if __name__ == '__main__':
    unittest.main()
