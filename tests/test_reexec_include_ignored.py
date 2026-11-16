"""tools/ci/reexec_include_ignored.py: a test that reruns its own executable selects the test, ignored or not."""
import subprocess
import sys
import textwrap
import unittest
from pathlib import Path

from tools.ci import reexec_include_ignored as reexec

ROOT = Path(__file__).resolve().parents[1]


class Reruns(unittest.TestCase):
    def test_each_form_gains_include_ignored_once_and_a_selecting_rerun_is_left(self):
        source = textwrap.dedent('''\
            fn inline() { std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", &t]).status(); }
            fn listed() {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        NAME,
                    ])
                    .output();
            }
            fn chained() { Command::new(current_exe()).arg("--exact").arg(NAME).spawn(); }
            fn built() -> Command {
                let mut c = Command::new(std::env::current_exe().unwrap());
                c.args(["--exact", "actor"]);
                c
            }
            fn selecting() { Command::new(current_exe()).args(["--ignored", "--exact", NAME]).status(); }
            fn other() { Command::new(current_exe()).arg("--version").status(); }
            ''').split("\n")
        self.assertEqual(len(reexec.missing(source)), 4)
        fixed = reexec.fix(list(source))
        text = "\n".join(fixed)
        self.assertEqual(reexec.missing("\n".join(fixed).split("\n")), [])
        self.assertIn('.args(["--include-ignored", "--exact", &t])', text)
        self.assertIn('        "--include-ignored",\n            "--exact",', text)
        self.assertEqual(text.count('"--include-ignored"'), 4)
        self.assertIn('.arg("--include-ignored").arg("--exact")', text)
        self.assertIn('c.args(["--include-ignored", "--exact", "actor"])', text)
        self.assertIn('.args(["--ignored", "--exact", NAME])', text)
        self.assertEqual(reexec.fix(fixed), fixed)

    def test_the_committed_tree_has_no_rerun_without_it(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools/ci/reexec_include_ignored.py"), "--check"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout[-2000:])


if __name__ == "__main__":
    unittest.main()
