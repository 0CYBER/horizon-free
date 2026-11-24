"""Exercise hygiene against real tracked contents, not source spelling."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tools/ci/check_repository_hygiene.py"
VENDOR = SCRIPT.parent / "vendor"
NOTICES = [
    "Showing " + "lines 1-933 of 1005",
    "Use " + "offset=934 to continue",
    "Output " + "truncated",
    "[" + "truncated",
    "lines " + "truncated",
]


class RepositoryHygieneTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write("clean.txt", "clean\n")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True).strip()

    def write(self, path, content, tracked=True):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content)
        if tracked:
            self.git("add", "--", path)

    def run_check(self, *, script=SCRIPT, pre_push=None):
        command = [sys.executable, str(script)]
        if pre_push is not None:
            command.append("--pre-push")
        return subprocess.run(command, cwd=self.root, input=pre_push, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)

    def test_clean_tree_and_untracked_or_ignored_notices(self):
        self.write(".github/workflows/ok.yml", "on: push\njobs: {}\n")
        self.write(".gitignore", "ignored.txt\n")
        self.write("ignored.txt", NOTICES[0], tracked=False)
        self.write("untracked.txt", NOTICES[1], tracked=False)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_every_notice_reports_tracked_file_and_line(self):
        for notice in NOTICES:
            with self.subTest(notice=notice):
                self.write("any-type.bin", "\0ordinary\n" + notice + "\n")
                result = self.run_check()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("any-type.bin:2:", result.stderr)
                self.assertIn(notice, result.stderr)

    def test_no_allow_path_can_hide_a_notice(self):
        for path in ["docs/quoted.md", "tests/test_fixture.py", "tools/ci/check.py"]:
            with self.subTest(path=path):
                self.write(path, NOTICES[0])
                result = self.run_check()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"{path}:1:", result.stderr)
                self.git("rm", "--cached", "-f", path)

    def test_all_workflow_files_are_parsed(self):
        self.write(".github/workflows/ok.yml", "on: push\njobs: {}\n")
        for path in [".github/workflows/ci.yml", ".github/workflows/other.yaml"]:
            with self.subTest(path=path):
                self.write(path, "name: [unfinished\n")
                result = self.run_check()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"{path}:", result.stderr)
                self.assertIn("invalid workflow YAML", result.stderr)
                self.write(path, "on: push\njobs: {}\n")
        self.write(".github/workflows/scalar.yml", "not-a-workflow\n")
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(".github/workflows/scalar.yml:1: workflow must be a YAML mapping", result.stderr)

    def test_pre_push_checks_each_committed_tip_not_clean_worktree(self):
        self.git("commit", "-qm", "clean")
        clean = self.git("rev-parse", "HEAD")
        self.write(".github/workflows/ci.yml", "name: [unfinished\n")
        self.git("commit", "-qm", "broken workflow")
        broken = self.git("rev-parse", "HEAD")
        self.write(".github/workflows/ci.yml", "on: push\njobs: {}\n")
        self.assertEqual(self.run_check().returncode, 0)
        refs = f"refs/heads/clean {clean} refs/heads/clean {'0' * 40}\n"
        refs += f"refs/heads/broken {broken} refs/heads/broken {'0' * 40}\n"
        result = self.run_check(pre_push=refs)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ci.yml:", result.stderr)
        self.assertIn("invalid workflow YAML", result.stderr)
        self.assertEqual(self.run_check(pre_push="").returncode, 0)
        self.assertEqual(self.run_check(pre_push=f"refs/heads/x {'0' * 40} refs/heads/x {clean}\n").returncode, 0)

    def test_missing_parser_fails_closed(self):
        isolated = self.root / "checker.py"
        shutil.copyfile(SCRIPT, isolated)
        result = self.run_check(script=isolated)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing vendored YAML parser", result.stderr)
        shutil.copytree(VENDOR, self.root / "vendor")
        with (self.root / "vendor/pyyaml-6.0.3.zip").open("ab") as archive:
            archive.write(b"corrupt")
        result = self.run_check(script=isolated)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)

    def test_offline_parser_without_site_packages(self):
        self.write(".github/workflows/ok.yml", "on: push\njobs: {}\n")
        command = [sys.executable, "-S", str(SCRIPT)]
        env = {**os.environ, "PYTHONPATH": ""}
        result = subprocess.run(command, cwd=self.root, env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.write(".github/workflows/ok.yml", "jobs: [unterminated\n")
        result = subprocess.run(command, cwd=self.root, env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid workflow YAML", result.stderr)

    def test_hook_validates_tips_and_preserves_public_tree_ref_input(self):
        self.write("tools/ci/check_repository_hygiene.py", SCRIPT.read_text())
        shutil.copytree(VENDOR, self.root / "tools/ci/vendor")
        self.write("tools/check_public_tree.py",
                   "import pathlib, sys\npathlib.Path('forwarded.txt').write_text(sys.stdin.read())\n")
        self.git("commit", "-qm", "hook fixture")
        tip = self.git("rev-parse", "HEAD")
        refs = f"refs/heads/x {tip} refs/heads/x {'0' * 40}\n"
        hook = SCRIPT.parents[2] / ".githooks/pre-push"
        result = subprocess.run(["sh", str(hook)], cwd=self.root, input=refs,
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / "forwarded.txt").read_text(), refs.rstrip("\n"))
        (self.root / "forwarded.txt").unlink()
        self.write("bad.md", NOTICES[0])
        self.git("commit", "-qm", "bad tip")
        bad = self.git("rev-parse", "HEAD")
        self.write("bad.md", "working tree fixed\n")
        result = subprocess.run(["sh", str(hook)], cwd=self.root,
                                input=f"refs/heads/x {bad} refs/heads/x {tip}\n",
                                text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("bad.md:1:", result.stderr)
        self.assertFalse((self.root / "forwarded.txt").exists())

    def test_removed_pattern_mutant_is_killed(self):
        # Change the real checker, then show the actual tracked-file test input
        # exposes its false negative. This is not a mock of git or the matcher.
        mutant = self.root / "mutant.py"
        source = SCRIPT.read_text()
        needle = '    r"Use offset=[0-9]+ to continue",\n'
        self.assertEqual(source.count(needle), 1)
        mutant.write_text(source.replace(needle, ""))
        shutil.copytree(VENDOR, self.root / "vendor")
        self.write("victim.txt", "safe\n" + NOTICES[1] + "\n")
        self.assertNotEqual(self.run_check().returncode, 0)
        self.assertEqual(self.run_check(script=mutant).returncode, 0)


if __name__ == "__main__":
    unittest.main()
