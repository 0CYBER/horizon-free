"""Protocol framing and state transitions of the offline capture proxy."""
import io
import json
import threading
import unittest

from tools import net_capture


class VarIntTests(unittest.TestCase):
    def test_a_five_byte_varint_decodes_and_a_sixth_byte_is_refused(self):
        self.assertEqual(net_capture.read_varint(b"\xff\xff\xff\xff\x0f", 0), (-1, 5))
        with self.assertRaisesRegex(ValueError, "varint too long"):
            net_capture.read_varint(b"\x80\x80\x80\x80\x80\x01", 0)


class StateTests(unittest.TestCase):
    def test_play_start_configuration_keeps_acknowledgment_in_play(self):
        out = io.StringIO()
        names = {
            ("play", "clientbound"): ["start_configuration"],
            ("play", "serverbound"): ["configuration_acknowledged"],
            ("configuration", "clientbound"): ["registry_data"],
            ("configuration", "serverbound"): ["finish_configuration"],
        }
        session = net_capture.Session(1, out, names, threading.Lock(), full=False)
        session.state["clientbound"] = session.state["serverbound"] = "play"
        session.record("clientbound", 0, b"")
        self.assertEqual(session.state, {"clientbound": "configuration", "serverbound": "play"})
        session.record("clientbound", 0, b"")
        session.record("serverbound", 0, b"")
        self.assertEqual(session.state, {"clientbound": "configuration", "serverbound": "configuration"})
        session.record("serverbound", 0, b"")
        entries = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertEqual([(e["state"], e["name"]) for e in entries], [
            ("play", "start_configuration"),
            ("configuration", "registry_data"),
            ("play", "configuration_acknowledged"),
            ("configuration", "finish_configuration"),
        ])
