from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import supervise


class TrialMergeTests(unittest.TestCase):
    def test_the_largest_compatible_set_avoids_every_conflicting_pair(self):
        conflicts = {frozenset(("a", "b")), frozenset(("b", "c"))}
        self.assertEqual(supervise.largest_compatible(["a", "b", "c", "d"], conflicts), ["a", "c", "d"])
        self.assertEqual(supervise.largest_compatible(["b", "a"], {frozenset(("a", "b"))}), ["b"])

    def test_plan_keeps_branches_that_merge_cleanly_together(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(supervise, "ROOT", Path(temp)):
            def run(*args):
                subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=temp, check=True,
                               capture_output=True)

            def commit(branch, name, text, start="base"):
                run("checkout", "-q", "-B", branch, start)
                Path(temp, name).write_text(text)
                run("add", name)
                run("commit", "-q", "-m", f"{branch} {name}")

            run("init", "-q", "-b", "base")
            run("config", "user.name", "t")  # supervise's own commits need an identity too (CI has none)
            run("config", "user.email", "t@t")
            Path(temp, "shared.txt").write_text("one\n")
            run("add", "shared.txt")
            run("commit", "-q", "-m", "start")
            commit("parity/a", "a.txt", "a\n")
            commit("parity/b", "shared.txt", "b\n")
            commit("parity/c", "shared.txt", "c\n")  # conflicts with parity/b only
            commit("parity/d", "d.txt", "d\n")
            commit("parity/e", "e.txt", "e\n", start="base")
            run("checkout", "-q", "base")
            commit("base", "shared.txt", "base\n")  # parity/b and parity/c now conflict with the base too
            chosen, head = supervise.plan("base", ["parity/a", "parity/b", "parity/c", "parity/d"], verbose=False)
            self.assertEqual(chosen, ["parity/a", "parity/d"])
            files = subprocess.run(["git", "ls-tree", "--name-only", head], cwd=temp, capture_output=True, text=True).stdout
            self.assertEqual(files.split(), ["a.txt", "d.txt", "shared.txt"])


if __name__ == "__main__":
    unittest.main()
