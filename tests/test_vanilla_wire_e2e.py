"""Pure-Python regression for the known-packs verdict in tools/vanilla_wire_e2e.py.

The rule compares what net_capture retained for the server's and the client's
select_known_packs. Large payloads are kept only as a 512-byte 'head' (no 'data'
key), so comparing .get('data') on both sides used to turn two different heads
into None != None -> False: a false parity pass.
"""
from pathlib import Path
import importlib.util
import sys
import types
import unittest
from unittest import mock


def _load_module():
    """Import the harness by path, stubbing the imports the rule itself never uses.

    The module pulls in a game/server harness at import time; none of that is
    needed to exercise rule_known_packs_as_vanilla. Stubs keep this test pure
    Python and leave no trace in sys.modules or sys.path for other test modules.
    """
    stubs = {}
    for name, attrs in (
            ('lab', ('LOCAL', 'VERSION', 'cargo_target', 'game_env')),
            ('net_capture', ('Session', 'load', 'start_background')),
            ('test_rust_parity', ('verify_artifacts',)),
            ('server_pack_e2e', ('Game',)),
            ('test_dedicated_server_parity', ('Server', 'free_port'))):
        module = types.ModuleType(name)
        for attr in attrs:
            setattr(module, attr, object())
        stubs[name] = module

    source = Path(__file__).resolve().parents[1] / 'tools' / 'vanilla_wire_e2e.py'
    spec = importlib.util.spec_from_file_location('vanilla_wire_e2e', source)
    loaded = importlib.util.module_from_spec(spec)
    saved_path = list(sys.path)
    with mock.patch.dict(sys.modules, stubs):
        try:
            spec.loader.exec_module(loaded)
        finally:
            # vanilla_wire_e2e prepends tools/ to sys.path on import; undo that.
            sys.path[:] = saved_path
    return loaded


vw = _load_module()


def exchange(offered, answered):
    """Two select_known_packs records: one from the server (s2c), one from the client (c2s)."""
    return [
        {'dir': 's2c', 'name': 'select_known_packs', **offered},
        {'dir': 'c2s', 'name': 'select_known_packs', **answered},
    ]


class KnownPacksVerdictTest(unittest.TestCase):
    def test_different_head_only_payloads_fail(self):
        """The repro: both records lack 'data', but their retained heads differ."""
        entries = exchange({'head': 'aaaaaaaa'}, {'head': 'bbbbbbbb'})
        with self.assertRaises(vw.Failed):
            vw.rule_known_packs_as_vanilla(entries)

    def test_matching_prefixes_cannot_prove_complete_payloads_equal(self):
        prefix = 'aa' * 512
        for client_size in (5000, 6000):
            with self.subTest(client_size=client_size):
                entries = exchange({'size': 5000, 'head': prefix},
                                   {'size': client_size, 'head': prefix})
                with self.assertRaises(vw.Failed):
                    vw.rule_known_packs_as_vanilla(entries)

    def test_matching_full_payloads_pass(self):
        entries = exchange({'data': '0a056d696e656372616674'}, {'data': '0a056d696e656372616674'})
        self.assertEqual(vw.rule_known_packs_as_vanilla(entries),
                         'answered with the packs offered (minecraft:core)')

    def test_different_full_payloads_fail(self):
        entries = exchange({'data': '0a056d696e656372616674'}, {'data': '0a046d6f6464'})
        with self.assertRaises(vw.Failed):
            vw.rule_known_packs_as_vanilla(entries)

    def test_missing_payload_on_one_side_fails(self):
        """A record with neither 'data' nor 'head' must not be treated as a match."""
        with self.assertRaises(vw.Failed):
            vw.rule_known_packs_as_vanilla(exchange({'data': '0a05'}, {}))
        with self.assertRaises(vw.Failed):
            vw.rule_known_packs_as_vanilla(exchange({'head': 'aaaaaaaa'}, {}))


if __name__ == '__main__':
    unittest.main()
