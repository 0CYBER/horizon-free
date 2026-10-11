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


if __name__ == "__main__":
    unittest.main()
