import os
import subprocess
import tempfile
from pathlib import Path
import sys
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]

from tools import port_regen as regen

LOCK = {"minecraft": {"version": "26.3", "manifest_sha1": "m3", "accepted_manifest_sha1": ["m3old"],
                      "accepted_manifest_note": "republished", "client_sha1": "c3", "server_sha1": "s3",
                      "java_major": 25},
        "vineflower": {"version": "1.12.0"}}


def manifest(version, client, server, java=25):
    return {"id": version, "downloads": {"client": {"sha1": client}, "server": {"sha1": server}},
            "javaVersion": {"majorVersion": java}}


class UntrackedStatusTests(unittest.TestCase):
    def test_a_newly_generated_untracked_output_is_regenerated(self):
        # Hooks export repository/index/config variables; cwd alone does not isolate Git.
        git_env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        with mock.patch.dict(os.environ, git_env, clear=True), tempfile.TemporaryDirectory(prefix='port-regen-') as folder:
            root = Path(folder)
            for args in (('init', '-q'), ('config', 'user.email', 'test@example.invalid'),
                         ('config', 'user.name', 'test')):
                subprocess.run(['git', *args], cwd=root, check=True, capture_output=True)
            (root / 'gen').mkdir()
            (root / 'gen/existing.rs').write_text('// old\n')
            (root / 'gen/new.rs').write_text('// one\n// two\n')
            subprocess.run(['git', 'add', 'gen/existing.rs'], cwd=root, check=True, capture_output=True)
            subprocess.run(['git', 'commit', '-qm', 'base'], cwd=root, check=True, capture_output=True)
            state, stat = regen.status(root, ['gen/existing.rs', 'gen/new.rs'])
        self.assertEqual(state, 'regenerated')
        self.assertEqual(stat, {'gen/new.rs': [2, 0]})


class PinTests(unittest.TestCase):
    def test_another_version_gets_a_new_block(self):
        pinned = regen.pinned_lock(LOCK, manifest("26.4", "c4", "s4", 26), "m4")
        self.assertEqual(pinned["minecraft"], {"version": "26.4", "manifest_sha1": "m4", "client_sha1": "c4",
                                               "server_sha1": "s4", "java_major": 26})
        self.assertEqual(pinned["vineflower"], LOCK["vineflower"])

    def test_the_same_version_keeps_its_block(self):
        for sha1 in ["m3", "m3old"]:
            self.assertIs(regen.pinned_lock(LOCK, manifest("26.3", "c3", "s3"), sha1), LOCK)

    def test_the_same_version_with_other_jars_is_repinned(self):
        self.assertEqual(regen.pinned_lock(LOCK, manifest("26.3", "c3b", "s3"), "m3b")["minecraft"]["client_sha1"],
                         "c3b")


class CompileErrorTests(unittest.TestCase):
    OUTPUT = """/x/mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java:45: error: cannot find symbol
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
                                                                                            ^
  symbol:   variable WORLD
  location: class RegistryLayer
/x/VanillaCommandParseOracle.java:12: error: incompatible types: int cannot be converted to String
2 errors
"""

    def test_names_the_symbols(self):
        self.assertEqual(regen.compile_errors(self.OUTPUT), [
            "ParityData.java:45: cannot find symbol: variable WORLD (class RegistryLayer)",
            "VanillaCommandParseOracle.java:12: incompatible types: int cannot be converted to String"])

    def test_nothing_from_other_failures(self):
        self.assertEqual(regen.compile_errors("expected the root's order to differ"), [])


class InterpreterTests(unittest.TestCase):
    def test_python_is_an_interpreter_that_runs(self):
        found = regen.python()
        done = subprocess.run([found, "-c", "import sys; sys.exit(sys.version_info < (3,))"],
                              capture_output=True)
        self.assertEqual(done.returncode, 0, f"{found} is not a Python 3 that runs")


class StatusTests(unittest.TestCase):
    def test_a_new_declared_output_is_regenerated_and_committed(self):
        git_env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        with mock.patch.dict(os.environ, git_env, clear=True), tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for args in (["init", "-q"], ["config", "user.email", "t@example.com"], ["config", "user.name", "t"]):
                regen.git(*args, cwd=root)
            (root / "research-lock.json").write_text("{}\n")
            (root / "gen.rs").write_text("a\n")
            regen.git("add", "--", "research-lock.json", "gen.rs", cwd=root)
            regen.git("commit", "-q", "-m", "base", cwd=root)
            (root / "new_output.rs").write_text("x\ny\nz\n")
            outputs = ["gen.rs", "new_output.rs"]
            state, stat = regen.status(root, outputs)
            self.assertEqual((state, stat), ("regenerated", {"new_output.rs": [3, 0]}))
            regen.commit(root, "26.4", {"g": {"status": state, "outputs": outputs, "stat": stat}})
            self.assertIn("new_output.rs", regen.git("ls-tree", "-r", "--name-only", "HEAD", cwd=root).split())


class UsedThroughVersionPortTests(unittest.TestCase):
    """The module is used through `tools/version_port.py`; run on its own it says so."""

    def test_it_says_how_it_is_used(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools/port_regen.py"), "--help"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("version_port.py", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
