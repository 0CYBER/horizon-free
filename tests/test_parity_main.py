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

    def test_bisect_names_a_merge_only_when_head_fails_and_the_base_passes(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity_main, "ROOT", Path(tmp)):
            git = lambda *arguments: parity_main.git(*arguments).stdout.strip()
            git("init", "-q")
            for n in range(4):
                git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty",
                    "-m", f"Merge pull request #{n} from x/y")
            commits = git("rev-list", "--reverse", "HEAD").split()
            last_run = Path(tmp) / ".local/parity-runner/last-run.json"
            last_run.parent.mkdir(parents=True)
            last_run.write_text('{"failed": ["music"]}')
            real_run = parity_main.subprocess.run
            for broken_from, blamed, code in ((None, None, 0), (0, None, 1), (2, "2", 1)):
                comments = []
                # git runs for real in the scratch repo; a `gh pr comment` is only recorded, never sent.
                fake_run = lambda command, **kwargs: (comments.append(command) if command[0] == "gh"
                                                      else real_run(command, **kwargs))
                with mock.patch.object(parity_main, "LAST_RUN", last_run), \
                     mock.patch.object(parity_main, "run_harnesses", lambda names, timeout: broken_from is not None
                                       and commits.index(git("rev-parse", "HEAD")) >= broken_from), \
                     mock.patch.object(parity_main.subprocess, "run", fake_run), \
                     mock.patch("builtins.print") as printed:
                    self.assertEqual(parity_main.bisect(commits[0], 60, comment=True), code,
                                     "0 only when the failures pass again at HEAD")
                if blamed:
                    self.assertEqual([command[3] for command in comments], [blamed])
                else:
                    self.assertEqual(comments, [], printed.call_args_list[-1])
                self.assertEqual(git("rev-parse", "HEAD"), commits[-1], "HEAD checked out again")

    def test_a_harness_a_commit_does_not_have_cannot_fail_there(self):
        with mock.patch.object(parity_main.subprocess, "run") as run:
            self.assertFalse(parity_main.run_harnesses(["not_a_harness_yet"], 60))
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
