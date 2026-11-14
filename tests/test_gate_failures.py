"""Gate failure reporting and the setup/check dependency contract."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

from tests.test_ci_concurrency import ROOT, yaml
from tools.ci.gate_summary import summary


class GateFailuresTests(unittest.TestCase):
    @unittest.skipIf(os.name == "nt", "executes the Bash gate step")
    def test_core_compiles_once_on_green_and_collects_errors_only_on_red(self):
        workflow = yaml.load((ROOT / ".github/workflows/dyna-ci.yml").read_text(), Loader=yaml.BaseLoader)
        step = next(s for s in workflow["jobs"]["verify"]["steps"] if s.get("id") == "core_build")
        for mode, expected in [("green", 0), ("red-build-success", 1), ("red-build-failure", 1)]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                cargo = root / "cargo"
                cargo.write_text(f"#!{sys.executable}\n"
                                 "import json, os, sys\n"
                                 "with open(os.environ['CALLS'], 'a') as f: f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                                 "mode = os.environ['MODE']\n"
                                 "sys.exit(0 if mode == 'green' or (sys.argv[1] == 'build' and mode == 'red-build-success') else 1)\n")
                cargo.chmod(0o755)
                env = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"],
                           JOBS="-j2", MODE=mode, CALLS=str(root / "calls"))
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"]],
                                        env=env, capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, expected, result.stderr)
                calls = [json.loads(line) for line in (root / "calls").read_text().splitlines()]
                self.assertEqual(calls[0], ["test", "-j2", "--profile", "test", "--workspace", "--lib",
                                            "--bins", "--tests", "--no-run", "--timings"])
                if mode == "green":
                    self.assertEqual(len(calls), 1)
                else:
                    self.assertEqual(calls[1:], [["build", "-j2", "--keep-going", "--profile", "test",
                                                   "--workspace", "--lib", "--bins", "--tests"]])

    def test_summary_lists_failures_not_skips_or_successes(self):
        self.assertEqual(summary({"lint": {"outcome": "failure"}, "test": {"outcome": "success"},
                                  "docs": {"outcome": "skipped"}}),
                         "### Failed gate steps\n\n- `lint`\n")

    def test_independent_checks_continue_but_failed_setup_stops_them(self):
        workflow = yaml.load((ROOT / ".github/workflows/dyna-ci.yml").read_text(), Loader=yaml.BaseLoader)
        for job in ("lint", "verify", "verify-static", "launcher"):
            steps = workflow["jobs"][job]["steps"]
            ids = {s.get("id") for s in steps}
            for step in steps:
                for dependency in re.findall(r"steps\.([\w-]+)\.", step.get("if", "")):
                    self.assertIn(dependency, ids, (job, step))
            checks = [s for s in steps[steps.index(next(s for s in steps if s.get("id") == "setup"))+1:] if s.get("name", "").startswith(("Lint ", "Test Rust", "Test launcher"))]
            for check in checks:
                self.assertNotIn("success()", check["if"])
                self.assertIn("!cancelled()", check["if"])
                self.assertIn(".outcome == 'success'", check["if"])
                self.assertNotIn("continue-on-error", check)
