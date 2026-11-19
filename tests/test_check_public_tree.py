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


TABLE = "".join(f'"minecraft:stone.{index}": 1,\n' for index in range(60))


class MergeScopeTests(unittest.TestCase):
    """A range answers for what its commits add, not for what a merge brings in from main."""

    def commit(self, directory, name, text):
        (Path(directory) / name).write_text(text)
        git(directory, "add", "-A")
        git(directory, "commit", "-qm", name)

    def branch_behind_main(self, directory):
        """`feature` forks from main's first commit; main then moves on to `moved`."""
        git(directory, "init", "-q", "-b", "main")
        self.commit(directory, "readme.md", "public\n")
        git(directory, "checkout", "-qb", "feature")
        self.commit(directory, "feature.md", "feature\n")
        git(directory, "checkout", "-q", "main")
        self.commit(directory, "moved.md", "main moved\n")

    def test_a_table_main_once_held_does_not_fail_a_branch_that_merged_that_commit(self):
        with TemporaryDirectory() as directory:
            self.branch_behind_main(directory)
            self.commit(directory, "old_table.json", TABLE)
            git(directory, "tag", "held")
            (Path(directory) / "old_table.json").unlink()
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "gone")
            git(directory, "checkout", "-q", "feature")
            git(directory, "merge", "-q", "--no-edit", "held")
            result = check(directory, "--range", "main..feature")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_table_a_merge_itself_adds_fails(self):
        with TemporaryDirectory() as directory:
            self.branch_behind_main(directory)
            git(directory, "checkout", "-q", "feature")
            git(directory, "merge", "-q", "--no-commit", "--no-ff", "main")
            (Path(directory) / "hidden.json").write_text(TABLE)
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "merge")
            result = check(directory, "--range", "main..feature")
            self.assertEqual(result.returncode, 1)
            self.assertIn("hidden.json", result.stderr)

    def test_a_table_in_a_conflict_resolution_fails(self):
        with TemporaryDirectory() as directory:
            git(directory, "init", "-q", "-b", "main")
            self.commit(directory, "data.json", "base\n")
            git(directory, "checkout", "-qb", "feature")
            self.commit(directory, "data.json", "feature\n")
            git(directory, "checkout", "-q", "main")
            self.commit(directory, "data.json", "main\n")
            git(directory, "checkout", "-q", "feature")
            subprocess.run(["git", "-C", directory, "merge", "-q", "main"], capture_output=True)
            (Path(directory) / "data.json").write_text(TABLE)
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "resolved")
            result = check(directory, "--range", "main..feature")
            self.assertEqual(result.returncode, 1)
            self.assertIn("data.json", result.stderr)

    def test_a_table_from_a_side_branch_fails_through_its_own_commit(self):
        with TemporaryDirectory() as directory:
            self.branch_behind_main(directory)
            git(directory, "checkout", "-qb", "side", "feature")
            self.commit(directory, "side.json", TABLE)
            (Path(directory) / "side.json").unlink()
            git(directory, "add", "-A")
            git(directory, "commit", "-qm", "removed again")
            git(directory, "checkout", "-q", "feature")
            git(directory, "merge", "-q", "--no-edit", "--no-ff", "side")
            result = check(directory, "--range", "main..feature")
            self.assertEqual(result.returncode, 1)
            self.assertIn("side.json", result.stderr)


if __name__ == "__main__":
    unittest.main()
