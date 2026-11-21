"""Pure-Python unit tests for tools/agent_control_e2e.py.

No game, real horizon-ctl binary or resource pack is launched: Cli's subprocess.run is faked and
run()'s scratch/driver/scenario factory points are monkeypatched.
"""
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import agent_control_e2e as e


def capture_cli(tool, **kwargs):
    """Return the words Cli appends to [ctl, --socket, socket], with subprocess.run faked."""
    cli = e.Cli('/bin/horizon-ctl', '/tmp/c.sock', Path('.'), {})
    calls = []

    def fake_run(argv, **kw):
        calls.append(argv)
        return subprocess.CompletedProcess(argv, 0, stdout='{}\n', stderr='')

    with mock.patch.object(e.subprocess, 'run', side_effect=fake_run):
        reply = cli(tool, **kwargs)
    assert reply == {}, reply
    assert len(calls) == 1, calls
    return calls[0][len(cli.base):]


class CliArgvTest(unittest.TestCase):
    """F4: the CLI argv must use the exact spellings declared in horizon-ctl's commands.rs."""

    def test_hyphenated_bool_flag(self):
        self.assertEqual(capture_cli('record_start', name='x', no_world=True),
                         ['record-start', 'x', '--no-world'])

    def test_record_replay_hyphenated_flags(self):
        self.assertEqual(capture_cli('record_replay', name='r', in_place=True, keep_world=True),
                         ['record-replay', 'r', '--in-place', '--keep-world'])

    def test_flag_with_value(self):
        self.assertEqual(capture_cli('record_replay', name='r', timeout=10),
                         ['record-replay', 'r', '--timeout', '10'])

    def test_face_is_a_positional_opt(self):
        self.assertEqual(capture_cli('interact', x=1, y=2, z=3, face='up'),
                         ['interact', '1', '2', '3', 'up'])
        self.assertEqual(capture_cli('look_at_block', x=1, y=2, z=3, face='north'),
                         ['look-at-block', '1', '2', '3', 'north'])

    def test_bool_flag_is_bare_and_false_is_skipped(self):
        self.assertEqual(capture_cli('launch', wait=False, offscreen=True),
                         ['launch', '--offscreen'])

    def test_positional_bool_opts_stay_bare_words(self):
        self.assertEqual(capture_cli('click', id='play', right=True, shift=True),
                         ['click', 'play', 'right', 'shift'])
        self.assertEqual(capture_cli('click_slot', slot=200, shift=True),
                         ['click-slot', '200', 'shift'])

    def test_lan_port_and_keywords(self):
        for commands, force in ((False, False), (True, False), (False, True), (True, True)):
            keywords = (['commands'] if commands else []) + (['force'] if force else [])
            for port in (None, 0, 25565):
                with self.subTest(commands=commands, force=force, port=port):
                    port_args = {} if port is None else {'port': port}
                    self.assertEqual(
                        capture_cli('lan', action='open', commands=commands, force=force, **port_args),
                        ['lan', 'open'] + ([] if port is None else [str(port)]) + keywords,
                    )

    def test_plain_positional_and_free_text_still_work(self):
        self.assertEqual(capture_cli('goto', x=1, y=64, z=-3), ['goto', '1', '64', '-3'])
        self.assertEqual(capture_cli('type', text='hello everyone', replace=True),
                         ['type', '--replace', 'hello everyone'])


class _Driver:
    def __init__(self, *args, **kwargs):
        self.calls = []

    def __call__(self, tool, **kwargs):
        self.calls.append(tool)
        return {}

    def close(self):
        pass


def run_with(driver_factory, scenario, temp_root, which='fake'):
    """Run e.run with a fake scratch dir/driver/scenario; return (result, scratch path)."""
    work = Path(tempfile.mkdtemp(dir=temp_root))
    scenarios = dict(e.SCENARIOS)
    scenarios[which] = scenario
    with mock.patch.object(e, 'scratch', return_value=(work, {})), \
         mock.patch.object(e, 'Cli', driver_factory), \
         mock.patch.object(e, 'Mcp', driver_factory), \
         mock.patch.object(e.time, 'sleep'), \
         mock.patch.dict(e.SCENARIOS, scenarios, clear=True):
        result = e.run('cli', Path('ctl'), Path('out'), Path('res'), which)
    return result, work


class RunCleanupTest(unittest.TestCase):
    """F2/F3: shutdown failures must not mask the scenario result or leak the scratch home."""

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_close_failure_does_not_mask_result_or_leak_scratch(self):
        class ClosingDriver(_Driver):
            def close(self):
                raise subprocess.TimeoutExpired('horizon-ctl mcp', 30)

        result, work = run_with(ClosingDriver, lambda d, out: 'ok', self.root)
        self.assertIs(result, True)
        self.assertFalse(work.exists(), f'scratch not removed: {work}')

    def test_malformed_reply_returns_false_instead_of_raising(self):
        class BadDriver(_Driver):
            def __call__(self, tool, **kwargs):
                raise json.JSONDecodeError('bad json', 'x', 0)

        result, work = run_with(BadDriver, lambda d, out: 'ok', self.root)
        self.assertIs(result, False)
        self.assertFalse(work.exists())

    def test_timeout_returns_false_instead_of_raising(self):
        class TimeoutDriver(_Driver):
            def __call__(self, tool, **kwargs):
                raise subprocess.TimeoutExpired('horizon-ctl', 600)

        result, work = run_with(TimeoutDriver, lambda d, out: 'ok', self.root)
        self.assertIs(result, False)
        self.assertFalse(work.exists())

    def test_scenario_failure_is_still_false(self):
        def failing(d, out):
            raise AssertionError('scenario failed')

        result, work = run_with(_Driver, failing, self.root)
        self.assertIs(result, False)
        self.assertFalse(work.exists())


if __name__ == '__main__':
    unittest.main()
