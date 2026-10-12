from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import parity


class ParityRunnerTests(unittest.TestCase):
    def test_dep_info_keeps_escaped_spaces(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "probe"
            Path(temp, "probe.d").write_text(f"{binary}: /a/x.rs /b/with\\ space.rs\n\n/a/x.rs:\n")
            self.assertEqual(parity.dep_info(binary), ["/a/x.rs", "/b/with space.rs"])

    def test_artifacts_follow_profile_and_stop_at_program_arguments(self):
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--release", "--example", "a", "--bin", "b"]),
                         [["example", "a", "release"], ["bin", "b", "release"]])
        self.assertEqual(parity.artifacts_of(["cargo", "run", "--profile", "fast", "--example", "c", "--", "--example", "x"]),
                         [["example", "c", "fast"]])
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--example", "d"]), [["example", "d", "dev"]])

    def test_java_sources_include_classes_pulled_from_the_sourcepath(self):
        with tempfile.TemporaryDirectory() as temp:
            sources, classes = Path(temp, "src"), Path(temp, "classes")
            (sources / "p").mkdir(parents=True)
            (classes / "p").mkdir(parents=True)
            for name in ("Oracle", "Helper"):
                (sources / "p" / f"{name}.java").write_text("")
            for name in ("Oracle", "Helper$Inner"):
                (classes / "p" / f"{name}.class").write_bytes(b"")
            found = parity.java_sources(["javac", "-sourcepath", str(sources), "-d", str(classes), str(sources / "p/Oracle.java")])
            self.assertEqual(sorted(Path(path).name for path in found), ["Helper.java", "Oracle.java", "Oracle.java"])

    def test_a_pass_goes_stale_when_a_dependency_changes(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            source = Path(temp, "a.rs")
            source.write_text("one")
            state = {"harnesses": {"x": {"status": "pass", "dependencies": ["a.rs"]}}, "hashes": {}}
            hasher = parity.Hasher(state["hashes"])
            state["harnesses"]["x"]["key"] = parity.key(state["harnesses"]["x"], hasher, "env")
            self.assertFalse(parity.stale("x", state, hasher, "env"))
            self.assertTrue(parity.stale("x", state, hasher, "other toolchain"))
            source.write_text("two!")
            self.assertTrue(parity.stale("x", state, hasher, "env"))

    def test_a_pass_from_another_worktree_counts_only_for_the_same_content(self):
        with tempfile.TemporaryDirectory() as temp:
            elsewhere, here, shared = Path(temp, "a"), Path(temp, "b"), Path(temp, "shared")
            for root in (elsewhere, here):
                root.mkdir()
                (root / "x.rs").write_text("same")
            with mock.patch.object(parity, "SHARED", shared):
                with mock.patch.object(parity, "ROOT", elsewhere):
                    record = {"status": "pass", "dependencies": ["x.rs"], "artifacts": []}
                    record["key"] = parity.key(record, parity.Hasher({}), "env")
                    parity.publish("x", record)
                with mock.patch.object(parity, "ROOT", here):
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertFalse(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))
                    self.assertTrue(state["harnesses"]["x"]["shared"])
                    (here / "x.rs").write_text("changed")
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertTrue(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))


class OracleReuseTests(unittest.TestCase):
    """tools/parity_shim.py replays a run only when its inputs and outputs are as recorded."""

    def setUp(self):
        import os, subprocess, sys
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.bin, self.shim = root / "bin", root / "shim"
        self.bin.mkdir(); self.shim.mkdir()
        # A fake oracle: copies its input to its output and counts its runs.
        fake = self.bin / "java"
        fake.write_text(f"#!{sys.executable}\nimport sys, pathlib\n"
                        "runs = pathlib.Path(sys.argv[1]).parent / 'runs'\n"
                        "runs.write_text(str(int(runs.read_text() or 0) + 1) if runs.exists() else '1')\n"
                        "pathlib.Path(sys.argv[2]).write_text(pathlib.Path(sys.argv[1]).read_text())\nprint('done')\n")
        fake.chmod(0o755)
        (self.shim / "java").symlink_to(Path(parity.__file__).with_name("parity_shim.py"))
        self.input, self.output = root / "work" / "in.txt", root / "work" / "out.txt"
        self.input.parent.mkdir()
        self.input.write_text("a")
        python = Path(sys.executable).parent
        self.env = dict(os.environ, PATH=f"{self.shim}:{self.bin}:{python}", PARITY_SHIM_REAL_PATH=str(self.bin),
                        PARITY_SHIM_CACHE=str(root / "cache"))
        self.subprocess = subprocess

    def tearDown(self):
        self.temp.cleanup()

    def oracle(self):
        result = self.subprocess.run(["java", str(self.input), str(self.output)], env=self.env, capture_output=True, text=True,
                                     stdin=self.subprocess.DEVNULL, check=True)
        self.assertEqual(result.stdout, "done\n")
        return int((self.input.parent / "runs").read_text())

    def test_reuses_until_an_input_or_output_changes(self):
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)  # replayed, stdout included
        self.input.write_text("a")  # same content, new mtime: still reusable
        self.assertEqual(self.oracle(), 1)
        self.input.write_text("b")
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.output.read_text(), "b")
        self.output.write_text("tampered")
        self.assertEqual(self.oracle(), 3)
        self.assertEqual(self.output.read_text(), "b")


if __name__ == "__main__":
    unittest.main()
