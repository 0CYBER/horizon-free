import argparse
import contextlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import entity_save_reload as harness


def binary_for(path):
    return harness.horizon_binary(argparse.Namespace(binary=path), Path('unused'))


class HorizonBinaryTests(unittest.TestCase):
    def test_relative_binary_is_resolved_against_the_callers_directory(self):
        # Each server runs with its world as the working directory, so the path must not stay relative.
        before = os.getcwd()
        with tempfile.TemporaryDirectory() as caller:
            os.chdir(caller)
            try:
                found = binary_for('target/fast/horizon-client')
                expected = Path(os.getcwd()) / 'target' / 'fast' / 'horizon-client'
            finally:
                os.chdir(before)
        self.assertTrue(found.is_absolute())
        self.assertEqual(found, expected)

    def test_absolute_binary_is_kept(self):
        binary = Path(os.path.abspath('horizon-client'))
        self.assertEqual(binary_for(str(binary)), binary)

    def test_build_failure_prints_log_path_and_contents(self):
        def fake_run(cmd, **kwargs):
            kwargs['stdout'].write('error[E0425]: cannot find value `missing`\n')
            return subprocess.CompletedProcess(cmd, 101)

        err = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp:
            build = Path(tmp)
            with mock.patch.object(harness.subprocess, 'run', side_effect=fake_run), \
                 contextlib.redirect_stderr(err):
                with self.assertRaises(subprocess.CalledProcessError) as ctx:
                    harness.horizon_binary(argparse.Namespace(binary=None), build)
            log_path = build / 'cargo-build.log'
            self.assertTrue(log_path.is_file())
            self.assertIn('cannot find value `missing`', log_path.read_text())

        self.assertEqual(ctx.exception.returncode, 101)
        self.assertIn(str(log_path), err.getvalue())
        self.assertIn('cannot find value `missing`', err.getvalue())


if __name__ == '__main__':
    unittest.main()
