"""tools/neoforge_oracle.py: the argument file and the links the server folder is built from (no server is started)."""

from pathlib import Path
import os
import tempfile
import unittest
from unittest import mock

from tools import neoforge_oracle as oracle


class ArgsFileTests(unittest.TestCase):
    def test_windows_uses_win_args(self):
        with mock.patch.object(oracle, 'WINDOWS', True):
            self.assertEqual(oracle.args_file(Path('home')).name, 'win_args.txt')

    def test_elsewhere_uses_unix_args(self):
        with mock.patch.object(oracle, 'WINDOWS', False):
            path = oracle.args_file(Path('home'))
        self.assertEqual(path.name, 'unix_args.txt')
        self.assertEqual(path.parent, Path('home') / 'libraries/net/neoforged/neoforge' / oracle.NEOFORGE)


class LinkTests(unittest.TestCase):
    def setUp(self):
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.root = Path(self._dir.name)
        (self.root / 'libs/sub').mkdir(parents=True)
        (self.root / 'libs/sub/a.jar').write_bytes(b'lib')
        (self.root / 'mod.jar').write_bytes(b'mod')

    def test_folder_shows_the_target_files(self):
        oracle.link(self.root / 'libraries', self.root / 'libs')
        self.assertEqual((self.root / 'libraries/sub/a.jar').read_bytes(), b'lib')

    def test_file_shows_the_target_bytes(self):
        oracle.link(self.root / 'linked.jar', self.root / 'mod.jar')
        self.assertEqual((self.root / 'linked.jar').read_bytes(), b'mod')

    @unittest.skipUnless(os.name == 'nt', 'Windows only')
    def test_windows_needs_no_symlink_privilege(self):
        with mock.patch.object(Path, 'symlink_to', side_effect=OSError(1314, 'privilege')):
            oracle.link(self.root / 'libraries', self.root / 'libs')
            oracle.link(self.root / 'linked.jar', self.root / 'mod.jar')
        self.assertEqual((self.root / 'libraries/sub/a.jar').read_bytes(), b'lib')
        self.assertEqual((self.root / 'linked.jar').read_bytes(), b'mod')

    @unittest.skipUnless(os.name == 'nt', 'Windows only')
    def test_windows_copies_when_linking_fails(self):
        with mock.patch.object(oracle.subprocess, 'run', return_value=mock.Mock(returncode=1)), \
                mock.patch.object(oracle.os, 'link', side_effect=OSError('cross-device')):
            oracle.link(self.root / 'libraries', self.root / 'libs')
            oracle.link(self.root / 'linked.jar', self.root / 'mod.jar')
        self.assertEqual((self.root / 'libraries/sub/a.jar').read_bytes(), b'lib')
        self.assertEqual((self.root / 'linked.jar').read_bytes(), b'mod')


class BuildCacheTests(unittest.TestCase):
    """The jar cache must be invalidated when the pinned NeoForge version changes."""

    def setUp(self):
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.root = Path(self._dir.name)
        src = self.root / 'src'
        (src / 'java').mkdir(parents=True)
        (src / 'resources').mkdir()
        java = src / 'java' / 'Exporter.java'
        java.write_text('class Exporter {}')
        # Keep the source well older than any jar the stubbed build writes, so the staleness
        # check (`jar.mtime > newest`) caches it deterministically regardless of clock granularity.
        os.utime(java, (1_000_000_000, 1_000_000_000))
        self.source = src
        self.compiles = []

    def _fake_run(self, cmd, **kwargs):
        self.compiles.append(list(cmd))
        return mock.Mock(returncode=0, stdout='', stderr='')

    def _fake_zip(self, path, *args, **kwargs):
        class FakeZip:
            def __enter__(self):
                Path(path).write_bytes(b'compiled-against-this-classpath')
                return self

            def __exit__(self, *exc):
                return None

            def write(self, *a, **k):
                pass

        return FakeZip()

    def _build(self, version, home, cp):
        with mock.patch.multiple(oracle, LOCAL=self.root / 'oracle', NEOFORGE=version), \
                mock.patch.object(oracle.subprocess, 'run', side_effect=self._fake_run), \
                mock.patch.object(oracle.zipfile, 'ZipFile', side_effect=self._fake_zip):
            return oracle.build(home, source=self.source, cp=cp)

    def test_switching_version_recompiles_into_a_distinct_jar(self):
        first = self._build('21.1.259', self.root / 'home-v1', 'cp-v1')
        after_first = len(self.compiles)
        second = self._build('99.9.999', self.root / 'home-v2', 'cp-v2')
        after_second = len(self.compiles)
        self.assertEqual(after_first, 1)
        self.assertEqual(after_second, 2, 'switching NeoForge must recompile against the new classpath')
        self.assertNotEqual(first, second, 'each version must get its own jar path')

    def test_repeated_build_at_one_version_stays_cached(self):
        first = self._build('21.1.259', self.root / 'home', 'cp')
        again = self._build('21.1.259', self.root / 'home', 'cp')
        self.assertEqual(first, again)
        self.assertEqual(len(self.compiles), 1, 'an unchanged build at one version must stay cached')


if __name__ == '__main__':
    unittest.main()
