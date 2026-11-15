"""tools/sync/dropped_upstream.py on a small fork/upstream history."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

TOOL = Path(__file__).resolve().parents[1] / "tools/sync/dropped_upstream.py"
spec = importlib.util.spec_from_file_location("dropped_upstream", TOOL)
dropped = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dropped)


class DroppedUpstreamTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        cwd = os.getcwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, cwd)
        self.git("init", "-q", "-b", "fork")
        for key, value in (("user.name", "t"), ("user.email", "t@t"), ("commit.gpgsign", "false")):
            self.git("config", key, value)
        self.write("kept.rs", "fn a() {}\n")
        self.write("carried.rs", "fn b() {}\n")
        self.write("removed.rs", "fn c() {}\n")
        self.commit("base")
        self.git("checkout", "-q", "-b", "upstream")
        self.write("kept.rs", "fn a() {}\nfn upstream_fix() {}\n")
        self.write("carried.rs", "fn b() {}\nfn upstream_feature() {}\n")
        self.git("rm", "-q", "removed.rs")
        self.commit("Upstream fix")
        self.git("checkout", "-q", "fork")
        self.write("kept.rs", "fn a() {}\nfn fork_hook() {}\n")
        self.commit("Fork hook")
        # The sync: conflicts resolved to the fork's side for kept.rs, upstream's deletion undone.
        subprocess.run(["git", "merge", "-q", "--no-commit", "--no-ff", "upstream"], capture_output=True)
        self.git("checkout", "fork", "--", "kept.rs")
        self.git("checkout", "fork~1", "--", "removed.rs")
        self.commit("Sync")
        self.merge = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout

    def write(self, name, text):
        (self.root / name).write_text(text)

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def test_a_file_resolved_to_the_fork_side_and_an_undone_deletion_are_flagged_a_carried_change_is_not(self):
        found, _ = dropped.rows(self.merge)
        self.assertEqual({(kind, path, lost, added) for kind, path, _, _, lost, added, _ in found},
                         {("dropped", "kept.rs", 1, 1), ("deleted-up", "removed.rs", 0, 0)})

    def test_markdown_names_the_upstream_commit_once(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            dropped.main([self.merge, "--markdown"])
        commit = self.git("rev-parse", "--short", "upstream").strip()
        rows = [line for line in out.getvalue().splitlines() if line.startswith(f"| `{commit}`")]
        self.assertEqual(len(rows), 1)
        self.assertIn("Upstream fix", rows[0])
        self.assertIn("dropped 1/1 kept.rs", rows[0])


if __name__ == "__main__":
    unittest.main()
