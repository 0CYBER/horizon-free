"""No Mojang data, network, JVM or harness execution is needed for root safety."""
import argparse
import contextlib
import hashlib
import importlib
import io
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
import zipfile

from tools import datafix_paths, pr_ready


class DatafixPaths(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.checkout = self.base / 'checkout'
        self.checkout.mkdir()
        self.shared = self.base / 'fixtures'
        self.shared.mkdir()
        self.stack = mock.patch.multiple(datafix_paths, ROOT=self.checkout, READ_ONLY=self.shared)
        self.stack.start()
        self.addCleanup(self.stack.stop)
        env = mock.patch.dict(os.environ, {}, clear=True)
        env.start()
        self.addCleanup(env.stop)

    def test_minecraft_input_does_not_select_output(self):
        (self.checkout / '.local').mkdir()
        (self.checkout / '.local/minecraft').symlink_to(self.shared, target_is_directory=True)
        self.assertEqual(datafix_paths.writable_root(), self.checkout / '.local/datafix')

    def test_symlinked_ancestor_refused(self):
        (self.checkout / '.local').symlink_to(self.shared, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'HORIZON_DATAFIX_ROOT'):
            datafix_paths.writable_root()
        self.assertEqual(list(self.shared.iterdir()), [])

    def test_datafix_symlink_refused(self):
        (self.checkout / '.local').mkdir()
        (self.checkout / '.local/datafix').symlink_to(self.shared, target_is_directory=True)
        with self.assertRaises(ValueError):
            datafix_paths.writable_root()

    def test_explicit_safe_root(self):
        os.environ['HORIZON_DATAFIX_ROOT'] = str(self.base / 'private')
        self.assertEqual(datafix_paths.writable_root(), self.base / 'private')

    def test_preflight_starts_no_subprocess_on_refusal(self):
        os.environ['HORIZON_DATAFIX_ROOT'] = str(self.shared / 'downloads')
        with mock.patch.object(pr_ready.subprocess, 'Popen') as runner:
            with self.assertRaisesRegex(ValueError, 'HORIZON_DATAFIX_ROOT'):
                step = pr_ready.Step('Parity', 'echo unexpected')
                step.datafix_selected = True
                pr_ready.run_script([step], 'local', False)
            runner.assert_not_called()

    def test_regular_file_and_unwritable_ancestor_refused(self):
        root = self.checkout / '.local/datafix'
        root.parent.mkdir()
        root.write_text('not a directory')
        with self.assertRaisesRegex(ValueError, 'HORIZON_DATAFIX_ROOT.*not a directory'):
            datafix_paths.writable_root()
        root.unlink()
        with mock.patch.object(datafix_paths.os, 'access', return_value=False):
            with self.assertRaisesRegex(ValueError, 'HORIZON_DATAFIX_ROOT.*not writable'):
                datafix_paths.writable_root()

    def test_protected_ancestor_root_refused(self):
        os.environ['HORIZON_DATAFIX_ROOT'] = str(self.base)
        (self.base / 'servers').symlink_to(self.shared / 'servers')
        with self.assertRaisesRegex(ValueError, 'HORIZON_DATAFIX_ROOT'):
            datafix_paths.writable_root()
        self.assertEqual(list(self.shared.iterdir()), [])

    def test_subprocess_receives_canonical_root(self):
        os.environ['HORIZON_DATAFIX_ROOT'] = 'private-datafix'
        step = pr_ready.Step('Parity', 'true')
        step.datafix_selected = True
        with mock.patch.object(datafix_paths.os, 'getcwd', return_value=str(self.checkout)), mock.patch.object(pr_ready.subprocess, 'Popen', side_effect=RuntimeError('intercepted')) as runner:
            with self.assertRaisesRegex(RuntimeError, 'intercepted'):
                pr_ready.run_script([step], 'local', False)
        self.assertEqual(runner.call_args.kwargs['env']['HORIZON_DATAFIX_ROOT'], str(self.checkout / 'private-datafix'))
        self.assertEqual(os.environ['HORIZON_DATAFIX_ROOT'], 'private-datafix')

    def test_unknown_dependencies_still_preflight_opt_in(self):
        args = argparse.Namespace(host='local', quick=False, harnesses=True, datafix=True)
        with mock.patch.object(pr_ready, 'affected_here', return_value=None):
            step = next(s for s in pr_ready.plan(args, 'origin/main', set(), {}, set(), {}) if s.name == 'Parity')
        self.assertTrue(step.datafix_selected)
        os.environ['HORIZON_DATAFIX_ROOT'] = str(self.shared)
        with mock.patch.object(pr_ready.subprocess, 'Popen') as runner:
            with self.assertRaisesRegex(ValueError, 'HORIZON_DATAFIX_ROOT'):
                pr_ready.run_script([step], 'local', False)
            runner.assert_not_called()

    def test_relative_override(self):
        os.environ['HORIZON_DATAFIX_ROOT'] = 'private-datafix'
        self.assertEqual(datafix_paths.writable_root(), Path('private-datafix').resolve())

    def test_descendant_symlinks_refused_without_writes(self):
        root = self.checkout / '.local/datafix'
        root.mkdir(parents=True)
        for name in ('servers', 'java', 'worlds', 'worlds-pre-1.17', 'generate.lock'):
            with self.subTest(name=name):
                link = root / name
                link.symlink_to(self.shared / name)
                with self.assertRaisesRegex(ValueError, 'HORIZON_DATAFIX_ROOT.*escapes'):
                    datafix_paths.writable_root()
                self.assertEqual(list(self.shared.iterdir()), [])
                link.unlink()

    def test_unrelated_local_and_remote_parity_do_not_preflight(self):
        os.environ['HORIZON_DATAFIX_ROOT'] = str(self.shared)
        for host, selected in [('local', False), ('macmini', True)]:
            step = pr_ready.Step('Parity', 'true')
            step.datafix_selected = selected
            with mock.patch.object(pr_ready.subprocess, 'Popen', side_effect=RuntimeError('runner reached')):
                with self.assertRaisesRegex(RuntimeError, 'runner reached'):
                    pr_ready.run_script([step], host, False)

    def test_author_plan_requires_datafix_opt_in(self):
        (self.checkout / '.local/minecraft').mkdir(parents=True)
        (self.checkout / '.local/horizon-client').mkdir()
        for opt_in in (False, True):
            args = argparse.Namespace(host='local', quick=False, harnesses=True, datafix=opt_in)
            output = io.StringIO()
            with mock.patch.object(pr_ready, 'ROOT', self.checkout), mock.patch.object(pr_ready, 'affected_here', return_value=['datafix']), contextlib.redirect_stderr(output):
                steps = pr_ready.plan(args, 'origin/main', set(), {}, set(), {})
            step = next(s for s in steps if s.name == 'Parity')
            self.assertEqual(step.datafix_selected, opt_in)
            self.assertEqual('--exclude datafix' in step.command, not opt_in)
            self.assertEqual(output.getvalue().count('datafix parity not run:'), 0 if opt_in else 1)

    def test_download_and_lock_destinations_are_private(self):
        # Script consumers import siblings directly and mutate sys.path. Keep
        # those mutations scoped so later unittest discovery cannot import a
        # tools/test_* module in place of a tests/test_* module with the same name.
        with_path = mock.patch.object(sys, 'path', [str(Path(__file__).resolve().parents[1] / 'tools'), *sys.path])
        with_path.start()
        self.addCleanup(with_path.stop)
        alias = mock.patch.dict(sys.modules, {'datafix_paths': datafix_paths})
        alias.start()
        self.addCleanup(alias.stop)
        # Import the actual consumers with the owning boundary selecting a temp root.
        for name in ('datafix_worlds', 'test_datafix_parity'):
            sys.modules.pop(name, None)
            self.addCleanup(sys.modules.pop, name, None)
        worlds = importlib.import_module('datafix_worlds')
        parity = importlib.import_module('test_datafix_parity')
        payload = io.BytesIO()
        with zipfile.ZipFile(payload, 'w') as jar:
            jar.writestr('dummy', 'synthetic jar')
        body = payload.getvalue()
        metadata = {'downloads': {'server': {'url': 'https://invalid.test/jar', 'sha1': hashlib.sha1(body).hexdigest()}}}
        with mock.patch.object(worlds, 'version_meta', return_value=metadata), mock.patch.object(worlds.urllib.request, 'urlopen', return_value=io.BytesIO(body)):
            path, _, _ = worlds.server_jar('1.8.9')
        self.assertEqual(path, self.checkout / '.local/datafix/servers/1.8.9/server.jar')
        self.assertEqual(path.read_bytes(), body)
        self.assertEqual(parity.WORLDS.parent / 'generate.lock', self.checkout / '.local/datafix/generate.lock')
        self.assertEqual(worlds.worlds_dir('1.18.2'), parity.WORLDS)
        flat = worlds.worlds_dir('1.12.2-flat') / '1.12.2-flat'
        flat.mkdir(parents=True)
        # Execute the real flat builder's kept-world path, without a server.
        with mock.patch.object(worlds, 'server_jar', return_value=(path, 1343, 340)):
            self.assertEqual(worlds.scene12.build_flat(worlds), [])
        self.assertEqual(flat, self.checkout / '.local/datafix/worlds-pre-1.17/1.12.2-flat')
        self.assertEqual(list(self.shared.iterdir()), [])
