"""--work must never clear a directory the caller supplied (issue #17299).

The real CLI is driven end to end with a synthetic --python-tree tar and a tiny fake
preparer, so no Java, game data or cargo build is involved. Each case records the bytes
of unrelated sentinels before and after the run: they must survive both success and the
validation failures, and only a fresh/absent work path may be created into.
"""

import hashlib
import json
import os
import shlex
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

TOOL = Path(__file__).resolve().parents[1] / "tools" / "compare_prepare.py"

# A preparer that writes the same two files no matter how often it is called, so a
# fully synthetic run is clean: identical output, no exporter noise, no rerun changes.
FAKE_PREPARER = '''#!/usr/bin/env python3
import hashlib, json
from pathlib import Path
out = Path.cwd() / ".local" / "horizon-client" / "26.3"
out.mkdir(parents=True, exist_ok=True)
data = b"fixed fixture bytes\\n"
(out / "data.txt").write_bytes(data)
(out / "horizon-resources.json").write_text(
    json.dumps({"data.txt": hashlib.sha256(data).hexdigest()}))
'''


def build_tree(base: Path) -> Path:
    """A tar with the two entries tree() extracts: tools/ and research-lock.json."""
    archive = base / "python-tree.tar"
    with tempfile.TemporaryDirectory() as td:
        src = Path(td)
        (src / "tools").mkdir()
        (src / "tools" / "prepare_native_client.py").write_text(FAKE_PREPARER)
        (src / "research-lock.json").write_text(json.dumps({"minecraft": {"version": "26.3"}}))
        with tarfile.open(archive, "w") as tf:
            tf.add(src / "tools", arcname="tools")
            tf.add(src / "research-lock.json", arcname="research-lock.json")
    return archive


def build_binary(base: Path):
    """A fake horizon-prepare that execs the same synthetic preparer as the Python runs."""
    prep = base / "fake_prepare.py"
    prep.write_text(FAKE_PREPARER)
    binary = base / "fake-prepare"
    binary.write_text("#!/bin/sh\nexec %s %s \"$@\"\n" %
                      (shlex.quote(sys.executable), shlex.quote(str(prep))))
    binary.chmod(binary.stat().st_mode | stat.S_IXUSR)
    return binary


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@unittest.skipIf(os.name == 'nt', 'executes a /bin/sh shebang wrapper and tar')
class WorkDirectoryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.jar = self.base / "dummy.jar"
        self.jar.write_bytes(b"not a real jar")
        self.tree = build_tree(self.base)
        self.binary = build_binary(self.base)

    def run_tool(self, work, *, tree=None, binary=None, jar=None, keep=True):
        command = [sys.executable, str(TOOL),
                   "--server-jar", str(jar or self.jar),
                   "--minecraft-dir", str(self.base),
                   "--python-tree", str(tree or self.tree),
                   "--binary", str(binary or self.binary),
                   "--work", str(work)]
        if keep:
            command.append("--keep")
        return subprocess.run(command, cwd=self.base, text=True, capture_output=True)

    def assert_sentinel_intact(self, sentinel, before):
        self.assertTrue(sentinel.is_file(), "an unrelated file was deleted")
        self.assertEqual(sha(sentinel), before, "an unrelated file was modified")

    # ---- successful fresh-work controls -------------------------------------

    def test_fresh_work_runs_and_keep_preserves_the_prepared_folders(self):
        for name in ("absent", "empty-existing"):
            with self.subTest(name=name):
                work = self.base / ("fresh-" + name)
                if name == "empty-existing":
                    work.mkdir()
                result = self.run_tool(work, keep=True)
                self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
                report = work / "report.json"
                self.assertTrue(report.is_file(), "the report was not written to --work")
                self.assertEqual(json.loads(report.read_text())["port_differences"], [])
                self.assertTrue((work / "python-1" / ".local" / "horizon-client" / "26.3" /
                                 "data.txt").is_file(), "--keep did not preserve the outputs")

    def test_fresh_work_without_keep_removes_only_the_owned_folders(self):
        work = self.base / "fresh-no-keep"
        result = self.run_tool(work, keep=False)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertFalse((work / "python-1").exists(), "the prepared folder was not cleaned")
        self.assertFalse((work / "rust").exists(), "the prepared folder was not cleaned")
        self.assertTrue((work / "report.json").is_file(), "cleanup removed unrelated run output")

    # ---- data-preservation regression matrix --------------------------------

    def test_nonempty_work_survives_a_failed_archive_extraction(self):
        """The issue's exact reproduction: notes must survive a bad tar, even with --keep."""
        work = self.base / "work"
        work.mkdir()
        sentinel = work / "unrelated-user-notes.txt"
        sentinel.write_text("KEEP ME: important user notes")
        before = sha(sentinel)
        bad_tar = self.base / "invalid-tree.tar"
        bad_tar.write_bytes(b"this is not a tar archive")

        result = self.run_tool(work, tree=bad_tar, keep=True)

        # Prove the data survived before inspecting the return code or any diagnostic wording,
        # so a red baseline pins deletion rather than the error text.
        self.assert_sentinel_intact(sentinel, before)
        self.assertNotEqual(result.returncode, 0, "a failed run reported success")
        self.assertIn("--work", result.stderr + result.stdout)
        self.assertEqual(sorted(p.name for p in work.iterdir()), ["unrelated-user-notes.txt"],
                         "the tool created or deleted entries in a nonempty --work")

    def test_nonempty_work_is_rejected_even_when_every_input_is_valid(self):
        work = self.base / "valid-input-work"
        work.mkdir()
        sentinel = work / "user-data.bin"
        sentinel.write_bytes(b"\x00\x01user data\xff")
        before = sha(sentinel)

        result = self.run_tool(work, keep=True)

        self.assert_sentinel_intact(sentinel, before)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--work", result.stderr + result.stdout)
        self.assertFalse((work / "python-1").exists(), "a preparer ran in a nonempty --work")

    def test_input_validation_fails_before_touching_work(self):
        work = self.base / "bad-input-work"
        work.mkdir()
        sentinel = work / "precious.txt"
        sentinel.write_text("do not delete")
        before = sha(sentinel)

        result = self.run_tool(work, jar=self.base / "missing.jar", keep=True)

        self.assert_sentinel_intact(sentinel, before)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--server-jar", result.stderr + result.stdout)
        self.assertEqual(sorted(p.name for p in work.iterdir()), ["precious.txt"])

    def test_unsafe_work_targets_are_rejected_without_deletion(self):
        file_work = self.base / "work-is-a-file"
        file_work.write_bytes(b"user owned file")
        target = self.base / "link-target"
        target.mkdir()
        link_sentinel = target / "keep.txt"
        link_sentinel.write_bytes(b"symlink target data")
        link = self.base / "work-is-a-symlink"
        link.symlink_to(target, target_is_directory=True)

        cases = [(file_work, file_work, b"user owned file"),
                 (link, link_sentinel, b"symlink target data")]
        for work, sentinel, expected in cases:
            with self.subTest(work=work.name):
                result = self.run_tool(work, keep=True)
                # Prove the path and its data survived before reading diagnostics.
                self.assertTrue(work.exists(), "the supplied work path itself was removed")
                self.assertTrue(sentinel.is_file(), "an unrelated file or symlink target was deleted")
                self.assertEqual(sentinel.read_bytes(), expected,
                                 "an unrelated file or symlink target was modified")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("--work", result.stderr + result.stdout)


if __name__ == "__main__":
    unittest.main()
