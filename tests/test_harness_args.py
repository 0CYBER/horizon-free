"""A command-oracle harness keeps the `--work-dir` its caller gave.

The harnesses append a default scratch directory when the caller named none.
`tools/test_command_exec_parity.py` reads that argument with argparse, where the
last `--work-dir` wins, so an extra pair appended after `--work-dir=/tmp/x`
would send the run somewhere the caller did not ask for.
"""
import contextlib
import importlib.util
import io
import json
import runpy
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

TOOLS = Path(__file__).resolve().parents[1] / 'tools'
_spec = importlib.util.spec_from_file_location('harness_args', TOOLS / 'harness_args.py')
harness_args = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(harness_args)

DEFAULT = Path('/scratch/default')


class EnsureWorkDirTest(unittest.TestCase):
    def test_one_argument_spelling_is_kept(self):
        argv = ['harness.py', '--work-dir=/tmp/asked-for', '--full-diagnostic']
        self.assertEqual(harness_args.ensure_work_dir(DEFAULT, argv), Path('/tmp/asked-for'))
        self.assertEqual(argv, ['harness.py', '--work-dir=/tmp/asked-for', '--full-diagnostic'])

    def test_separate_spelling_is_kept(self):
        argv = ['harness.py', '--work-dir', '/tmp/asked-for']
        self.assertEqual(harness_args.ensure_work_dir(DEFAULT, argv), Path('/tmp/asked-for'))
        self.assertEqual(argv, ['harness.py', '--work-dir', '/tmp/asked-for'])

    def test_default_is_appended_when_none_is_given(self):
        argv = ['harness.py', '--reference', '/tmp/vanilla.jsonl']
        self.assertEqual(harness_args.ensure_work_dir(DEFAULT, argv), DEFAULT)
        self.assertEqual(
            argv,
            ['harness.py', '--reference', '/tmp/vanilla.jsonl', '--work-dir', str(DEFAULT)],
        )

    def test_script_path_is_not_an_argument(self):
        argv = ['--work-dir/not-an-argument.py']
        harness_args.ensure_work_dir(DEFAULT, argv)
        self.assertEqual(argv, ['--work-dir/not-an-argument.py', '--work-dir', str(DEFAULT)])

    def test_the_last_one_wins_as_in_argparse(self):
        argv = ['harness.py', '--work-dir', '/tmp/first', '--work-dir=/tmp/second']
        self.assertEqual(harness_args.ensure_work_dir(DEFAULT, argv), Path('/tmp/second'))


class FreshCaptureHarnessTest(unittest.TestCase):
    def test_a_passing_automatic_run_removes_its_capture_and_a_failing_one_keeps_it(self):
        for name, folder in [('loot_kill_sheep_state', 'h036'), ('loot_mine_location_block', 'h079'),
                             ('warden_owned_memories', 'h029')]:
            with self.subTest(harness=name), tempfile.TemporaryDirectory() as scratch, contextlib.chdir(scratch):
                legacy = Path('.local') / folder / 'capture' / 'vanilla.jsonl'
                legacy.parent.mkdir(parents=True)
                legacy.write_bytes(b'earlier oracle evidence\n')
                captures = []
                mismatch = []
                driver = types.ModuleType('test_command_exec_parity')
                driver.corpus_module = types.SimpleNamespace()

                def capture():
                    # Isolate the expensive Java/game processes, not the real wrapper or its filesystem work.
                    directory = harness_args.require_work_dir().resolve()
                    count = json.loads((directory / 'declaration.json').read_text())['records']
                    records = (f'{{"capture":{len(captures) + 1}}}\n' * count).encode()
                    (directory / 'vanilla.jsonl').write_bytes(records)
                    (directory / 'rust.jsonl').write_bytes(records.replace(b'}', b',"x":1}') if mismatch else records)
                    captures.append((directory, records))

                driver.main = capture
                script = TOOLS / f'test_{name}_parity.py'
                explicit = Path('chosen capture').resolve()
                with mock.patch.dict(sys.modules, {'harness_args': harness_args,
                                                  'test_command_exec_parity': driver}), \
                        mock.patch.object(sys, 'path', sys.path.copy()), \
                        contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    for extra in [[], [], [f'--work-dir={explicit}']]:
                        with mock.patch.object(sys, 'argv', [str(script), *extra]), \
                                self.assertRaises(SystemExit) as exit:
                            runpy.run_path(str(script), run_name='__main__')
                        self.assertEqual(exit.exception.code, 0)
                    # A run whose records differ fails and keeps its capture.
                    mismatch.append(True)
                    with mock.patch.object(sys, 'argv', [str(script)]), \
                            self.assertRaises((AssertionError, SystemExit)) as failed:
                        runpy.run_path(str(script), run_name='__main__')
                    self.assertNotEqual(getattr(failed.exception, 'code', 1), 0)
                self.assertEqual(len(captures), 4)
                self.assertNotEqual(captures[0][0], captures[1][0])
                self.assertEqual(captures[2][0], explicit)
                self.assertEqual(legacy.read_bytes(), b'earlier oracle evidence\n')
                # The passing automatic runs leave nothing behind.
                self.assertFalse(captures[0][0].exists())
                self.assertFalse(captures[1][0].exists())
                # The folder the caller named stays, with what the run wrote.
                directory, records = captures[2]
                self.assertEqual((directory / 'vanilla.jsonl').read_bytes(), records)
                self.assertEqual((directory / 'rust.jsonl').read_bytes(), records)
                self.assertEqual(json.loads((directory / 'strict-report.json').read_text())['differences'], [])
                # The failing automatic run's folder stays as evidence.
                directory, records = captures[3]
                self.assertEqual((directory / 'vanilla.jsonl').read_bytes(), records)
                self.assertNotEqual((directory / 'rust.jsonl').read_bytes(), records)


if __name__ == '__main__':
    unittest.main()
