"""Self-check orchestration contracts; all check execution is mocked, no Cargo."""
import contextlib
import io
import unittest
import subprocess
import tempfile
from pathlib import Path
from argparse import Namespace
from unittest.mock import patch

from tools import pr_ready


class RevisionOrchestration(unittest.TestCase):
    def test_plan_runs_real_repository_hygiene_even_for_python_only_delta(self):
        steps = pr_ready.plan(Namespace(host="local", quick=True, harnesses=False), "HEAD", {"tools/ci/ignored_tests.py"}, {}, set(), {})
        hygiene = next(step for step in steps if step.name == "Repository hygiene")
        self.assertIsNone(hygiene.skip)
        self.assertEqual(hygiene.where, "here")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", directory], check=True)
            # Reuse the real guard/parser while the guard inspects only this private repo.
            (root / "tools").symlink_to(pr_ready.ROOT / "tools", target_is_directory=True)
            bad = root / "bad.txt"
            bad.write_text("[" + "truncated" + "]\n")
            subprocess.run(["git", "add", "bad.txt"], cwd=root, check=True)
            failed = subprocess.run(hygiene.command, cwd=root, shell=True, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn("repository hygiene", failed.stderr)
            bad.write_text("complete authored content\n")
            passed = subprocess.run(hygiene.command, cwd=root, shell=True, capture_output=True, text=True)
            self.assertEqual(passed.returncode, 0, passed.stderr)

    def test_starting_identity_survives_concurrent_commit_and_dirty_commit(self):
        for status in ("", " M tracked.rs\n"):
            for ending in (("b" * 40, "", ""), ("a" * 40, status, "changed contents")):
                start = ("a" * 40, status, "")
                calls = []
                def snapshot():
                    calls.append("snapshot")
                    return start if calls.count("snapshot") == 1 else ending
                def selected(_):
                    calls.append("selection")
                    self.assertEqual(calls[0], "snapshot")
                    return set()
                def checks(*_):
                    calls.append("checks")
                with patch.object(pr_ready, "revision_state", side_effect=snapshot), \
                        patch.object(pr_ready.parity, "changed_files", side_effect=selected), \
                        patch.object(pr_ready, "git", return_value="mergebase"), \
                        patch.object(pr_ready, "line_counts", return_value={}), \
                        patch.object(pr_ready, "plan", return_value=[pr_ready.Step("Cargo.lock", where="git")]), \
                        patch.object(pr_ready, "lock_problem", return_value=None), \
                        patch.object(pr_ready, "run_checks", side_effect=checks), \
                        patch.object(pr_ready, "rust_version", return_value="1.99.0"), \
                        patch.object(pr_ready, "old_rust", return_value=None), \
                        patch.object(pr_ready, "suggested_label", return_value="skip-release"), \
                        patch.object(pr_ready.release_train, "exporters", return_value=[]), \
                        patch("sys.argv", ["pr_ready.py", "--quick"]), \
                        contextlib.redirect_stdout(io.StringIO()) as output, \
                        contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(pr_ready.main(), 0)
                header = output.getvalue().splitlines()[0]
                self.assertIn("commit `" + "a" * 40 + "`", header)
                self.assertNotIn("b" * 40, header)
                self.assertEqual(" dirty" in header, bool(status))
                self.assertIn("changed during run", header)
                self.assertEqual(calls, ["snapshot", "selection", "checks", "snapshot"])
