"""Unit tests for tools/vanilla_login.py, using a scripted Connection stand-in."""
import contextlib
import io
import sys
import unittest
from unittest import mock

from tools import lab, vanilla_login


class FakeSocket:
    def __init__(self):
        self.timeouts = []
        self.closed = False

    def settimeout(self, value):
        self.timeouts.append(value)

    def close(self):
        self.closed = True


class FakeConnection:
    """Replays scripted (id, payload) packets and records everything sent."""

    def __init__(self, *receives):
        self.receives = list(receives)
        self.sent = []
        self.sock = FakeSocket()
        self.threshold = -1

    def send(self, packet_id, payload=b''):
        self.sent.append((packet_id, payload))

    def receive(self):
        if not self.receives:
            raise EOFError('server closed the connection')
        item = self.receives.pop(0)
        if isinstance(item, BaseException):
            raise item
        return item


# Login/compression, login finished, select known packs, finish configuration.
OPEN = [(3, lab.varint(16)), (2, b''), (15, b''), (3, b'')]


class VanillaLoginTests(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.object(vanilla_login.lab, 'version_info',
                                    return_value={'protocol_version': 777})
        patcher.start()
        self.addCleanup(patcher.stop)

    def join_with(self, fake, seconds=1):
        with mock.patch.object(vanilla_login, 'Connection', return_value=fake):
            vanilla_login.join('127.0.0.1', 25565, 'Tester', seconds)

    def test_play_keep_alive_is_answered_with_the_same_payload(self):
        payload = b'\x00\x01\x02\x03\x04\x05\x06\x07'
        fake = FakeConnection(*OPEN, (0, b'ignored'), (45, payload))
        self.join_with(fake)
        self.assertEqual(fake.sent.count((28, payload)), 1)
        self.assertEqual(fake.sock.timeouts, [1.0])
        self.assertTrue(fake.sock.closed)

    def test_cookie_request_is_answered_with_an_absent_payload(self):
        # A request has only an identifier; a response also needs the optional-payload flag.
        cookie = b'\x0fminecraft:probe'
        fake = FakeConnection((3, lab.varint(16)), (2, b''),
                              (15, b''), (0, cookie), (3, b''))
        self.join_with(fake)
        self.assertIn((1, b'\x0fminecraft:probe\x00'), fake.sent)

    def test_resource_pack_push_raises_a_clear_error(self):
        fake = FakeConnection((3, lab.varint(16)), (2, b''),
                              (15, b''), (9, b'pack'))
        with self.assertRaisesRegex(RuntimeError, 'resource_pack_push'):
            self.join_with(fake)

    def test_code_of_conduct_raises_instead_of_waiting_for_configuration(self):
        # The server waits for packet 9 acceptance after sending packet 20.
        # This bare client must not silently wait or accept unseen terms.
        fake = FakeConnection((3, lab.varint(16)), (2, b''),
                              (15, b''), (20, b'server terms'), (3, b''))
        with self.assertRaisesRegex(RuntimeError, 'unhandled configuration packet 20.*code_of_conduct'):
            self.join_with(fake)
        self.assertNotIn((9, b''), fake.sent)

    def test_non_brand_custom_payload_raises_a_clear_error(self):
        channel = b'mod:query'
        data = lab.varint(len(channel)) + channel + b'\x00'
        fake = FakeConnection((3, lab.varint(16)), (2, b''),
                              (15, b''), (1, data))
        with self.assertRaisesRegex(RuntimeError, 'custom_payload'):
            self.join_with(fake)

    def test_brand_custom_payload_needs_no_answer(self):
        channel = b'minecraft:brand'
        data = lab.varint(len(channel)) + channel + b'vanilla'
        fake = FakeConnection((3, lab.varint(16)), (2, b''),
                              (15, b''), (1, data), (3, b''))
        self.join_with(fake)
        self.assertTrue(fake.sock.closed)

    def test_more_than_25_seconds_is_rejected(self):
        argv = ['vanilla_login.py', '--seconds', '26', 'Tester']
        with mock.patch.object(sys, 'argv', argv), mock.patch.object(vanilla_login, 'join') as join:
            with contextlib.redirect_stderr(io.StringIO()) as error:
                with self.assertRaises(SystemExit) as raised:
                    vanilla_login.main()
        self.assertEqual(raised.exception.code, 2)
        self.assertIn('at most 25', error.getvalue())
        join.assert_not_called()

    def test_exactly_25_seconds_is_accepted(self):
        argv = ['vanilla_login.py', '--seconds', '25', 'Tester']
        with mock.patch.object(sys, 'argv', argv), mock.patch.object(vanilla_login, 'join') as join:
            self.assertEqual(vanilla_login.main(), 0)
        join.assert_called_once_with('127.0.0.1', 25565, 'Tester', 25.0)


if __name__ == '__main__':
    unittest.main()
