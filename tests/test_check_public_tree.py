"""The public-tree check scans HEAD's history, not every branch's."""
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tools/check_public_tree.py"


def git(repo, *args):
    subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
                   check=True, capture_output=True)


def check(repo, *args):
    return subprocess.run(["python3", str(SCRIPT), *args], cwd=repo, capture_output=True, text=True)


class HistoryScopeTests(unittest.TestCase):
    def repo_with_artifact_on_a_branch(self, directory):
        git(directory, "init", "-q", "-b", "main")
        (Path(directory) / "readme.md").write_text("public\n")
        git(directory, "add", "readme.md")
        git(directory, "commit", "-qm", "main")
        git(directory, "checkout", "-qb", "other")
        (Path(directory) / "Leak.java").write_text("class Leak {}\n")
        git(directory, "add", "Leak.java")
        git(directory, "commit", "-qm", "artifact")
        git(directory, "checkout", "-q", "main")

    def test_an_artifact_only_on_another_branch_does_not_fail_head_history(self):
        with TemporaryDirectory() as directory:
            self.repo_with_artifact_on_a_branch(directory)
            result = check(directory, "--history")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_an_artifact_in_head_history_fails(self):
        with TemporaryDirectory() as directory:
            self.repo_with_artifact_on_a_branch(directory)
            git(directory, "merge", "-q", "--no-edit", "other")
            result = check(directory, "--history")
            self.assertEqual(result.returncode, 1)
            self.assertIn("Leak.java", result.stderr)

    def test_the_jfr_summary_is_allowed_and_another_java_tool_is_not(self):
        with TemporaryDirectory() as directory:
            git(directory, "init", "-q", "-b", "main")
            tools = Path(directory) / "tools"
            tools.mkdir(parents=True)
            (tools / "JfrSummary.java").write_text("class JfrSummary {}\n")
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "oracle")
            self.assertEqual(check(directory, "--history").returncode, 0)
            (tools / "Other.java").write_text("class Other {}\n")
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "other")
            self.assertEqual(check(directory, "--history").returncode, 1)


if __name__ == "__main__":
    unittest.main()
