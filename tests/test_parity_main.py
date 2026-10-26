"""tools/parity_main.py: the base of a parity-main run, and the bisect after a failure."""
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import parity_main


class ParityMainTests(unittest.TestCase):
    def test_first_bad_finds_the_first_failing_merge(self):
        commits = list("abcdefgh")  # h failed; the base before a passed
        for culprit in commits:
            tried = []
            fails = lambda commit: tried.append(commit) or commit >= culprit
            self.assertEqual(parity_main.first_bad(commits, fails), culprit)
            self.assertLessEqual(len(tried), 3, "log2 of 8 merges")
        self.assertEqual(parity_main.first_bad(["a"], lambda commit: self.fail("nothing to try")), "a")

    def test_base_is_the_newest_green_run_head_contains(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity_main, "ROOT", Path(tmp)):
            git = lambda *arguments: parity_main.git(*arguments).stdout.strip()
            git("init", "-q")
            for n in range(3):
                git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", str(n))
            head, parent, grandparent = git("rev-parse", "HEAD"), git("rev-parse", "HEAD^1"), git("rev-parse", "HEAD~2")
            git("checkout", "-q", "-b", "side", grandparent)
            git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "side")
            side = git("rev-parse", "HEAD")
            git("checkout", "-q", head)
            self.assertEqual(parity_main.base(runs=[side, grandparent, parent]), grandparent,
                             "a run on a commit HEAD doesn't contain is skipped")
            self.assertEqual(parity_main.base(runs=[]), parent, "no green run yet: the last merge")
            self.assertEqual(parity_main.base(runs=[head]), head)

    def test_a_harness_a_commit_does_not_have_cannot_fail_there(self):
        with mock.patch.object(parity_main.subprocess, "run") as run:
            self.assertFalse(parity_main.run_harnesses(["not_a_harness_yet"], 60))
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
