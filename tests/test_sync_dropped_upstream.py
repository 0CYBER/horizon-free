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
        self.write("moved_away.rs", "".join(f"fn moved_{n}() {{}}\n" for n in range(20)))
        (self.root / "game").mkdir()
        self.write("game/tick.rs", "fn tick() {}\n")
        self.write("other.rs", "fn other() {}\nfn shared_helper_with_a_long_enough_name() {}\n")
        self.write("lone.rs", "fn lone() {}\n")
        (self.root / "icon.png").write_bytes(b"\x89PNG\0old")
        self.commit("base")
        self.git("checkout", "-q", "-b", "upstream")
        self.write("kept.rs", "fn a() {}\nfn upstream_fix() {}\n")
        self.write("carried.rs", "fn b() {}\nfn upstream_feature() {}\n")
        self.git("rm", "-q", "removed.rs", "moved_away.rs")
        # Upstream's added line also exists in an unrelated fork file: that must not count as carried.
        self.write("game/tick.rs", "fn tick() {}\nfn other() {}\n")
        self.write("lone.rs", "fn lone() {}\nfn shared_helper_with_a_long_enough_name() {}\n")
        (self.root / "icon.png").write_bytes(b"\x89PNG\0new")
        self.commit("Upstream fix")
        self.git("checkout", "-q", "fork")
        self.write("kept.rs", "fn a() {}\nfn fork_hook() {}\n")
        (self.root / "game/area").mkdir()
        self.git("mv", "game/tick.rs", "game/area/tick.rs")
        self.write("game/area/tick.rs", "// rewritten by the fork\nfn fork_tick() {}\n")
        self.git("mv", "moved_away.rs", "kept_here.rs")
        self.git("rm", "-q", "lone.rs")
        self.commit("Fork hook")
        # The sync: kept.rs and icon.png resolved to the fork's side, both upstream deletions undone (one at the
        # fork's new path), tick.rs taken as the fork rewrote it.
        subprocess.run(["git", "merge", "-q", "--no-commit", "--no-ff", "upstream"], capture_output=True)
        for path in ("kept.rs", "icon.png", "kept_here.rs", "game/area/tick.rs"):
            self.git("checkout", "fork", "--", path)
        if (self.root / "lone.rs").exists():
            self.git("rm", "-q", "lone.rs")
        self.git("checkout", "fork~1", "--", "removed.rs")
        if (self.root / "game/tick.rs").exists():
            self.git("rm", "-q", "game/tick.rs")
        self.commit("Sync")
        self.merge = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout

    def write(self, name, text):
        (self.root / name).write_text(text)

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def test_every_dropped_change_is_listed_and_a_carried_one_is_not(self):
        found, _ = dropped.rows(self.merge)
        self.assertEqual({(kind, path, fork_path) for kind, path, fork_path, *_ in found},
                         {("dropped", "kept.rs", "kept.rs"), ("deleted-up", "removed.rs", "removed.rs"),
                          ("deleted-up", "moved_away.rs", "kept_here.rs"), ("binary", "icon.png", "icon.png"),
                          ("guessed", "game/tick.rs", "game/area/tick.rs"),
                          # Its added line is in an unrelated file: still listed, as a guess.
                          ("guessed", "lone.rs", "other.rs")})

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
