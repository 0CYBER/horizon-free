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
        for mode, expected in [("green", 0), ("red-build-success", 1), ("red-build-failure", 1),
                               ("sccache-kill-recovered", 0), ("rustc-kill-recovered", 0),
                               ("linker-kill-recovered", 0), ("kill-twice", 1)]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                cargo = root / "cargo"
                cargo.write_text(f"#!{sys.executable}\n"
                                 "import json, os, sys\n"
                                 "with open(os.environ['CALLS'], 'a') as f: f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                                 "assert os.environ['RUSTC_WRAPPER'] == os.environ['GITHUB_WORKSPACE'] + '/tools/ci/test-compile-lock.sh'\n"
                                 "assert os.environ['HORIZON_TEST_RUSTC_WRAPPER'] == '/configured/compiler-cache.sh'\n"
                                 "mode = os.environ['MODE']\n"
                                 "if sys.argv[1] == 'test':\n"
                                 "    if mode == 'green' or ('-j1' in sys.argv and mode.endswith('-recovered')): sys.exit(0)\n"
                                 "    if 'kill' in mode:\n"
                                 "        if mode.startswith('linker'):\n"
                                 "            print('collect2: fatal error: ld terminated with signal 9 [Killed]', file=sys.stderr)\n"
                                 "        else:\n"
                                 "            print('signal: 9, SIGKILL: kill' if mode.startswith('rustc') else 'sccache: Compile terminated by signal 9', file=sys.stderr)\n"
                                 "    print('error: could not compile `horizon-client-lib` (lib test)', file=sys.stderr)\n"
                                 "    sys.exit(1)\n"
                                 "sys.exit(0 if mode == 'red-build-success' else 1)\n")
                cargo.chmod(0o755)
                env = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"],
                           JOBS="-j2", MODE=mode, CALLS=str(root / "calls"),
                           GITHUB_WORKSPACE=str(ROOT), RUSTC_WRAPPER="/configured/compiler-cache.sh")
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"]],
                                        env=env, capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, expected, result.stderr)
                calls = [json.loads(line) for line in (root / "calls").read_text().splitlines()]
                self.assertEqual(calls[0], ["test", "-j2", "--profile", "test", "--workspace", "--lib",
                                            "--bins", "--tests", "--no-run", "--timings"])
                killed = "kill" in mode
                if killed:
                    self.assertEqual(calls[1], ["test", "-j1", "--profile", "test", "--workspace",
                                                "--lib", "--bins", "--tests", "--no-run", "--timings"])
                    self.assertIn("::warning::", result.stdout)
                    self.assertIn("Failed units: error: could not compile `horizon-client-lib` (lib test)", result.stdout)
                else:
                    self.assertNotIn("::warning::", result.stdout)
                if expected == 0:
                    self.assertEqual(len(calls), 2 if killed else 1)
                    self.assertNotIn("::error::", result.stdout)
                else:
                    self.assertEqual(calls[2 if killed else 1:], [["build", "-j2", "--keep-going", "--profile", "test",
                                                                 "--workspace", "--lib", "--bins", "--tests"]])
                    self.assertIn("::error::The test build failed", result.stdout)
                    # A failure the keep-going build cannot reproduce is named as one, not left silent.
                    self.assertEqual("does not clear the failed test build" in result.stdout,
                                     mode == "red-build-success")

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
