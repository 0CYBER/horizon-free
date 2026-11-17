"""Regression for profile_shared_json.run() with a verbose profiled child.

A profiled executable that writes more than a pipe buffer to stderr must not
deadlock the measurement (the old code waited with wait4 before draining the
stderr PIPE). The per-child wait4 CPU/RSS accounting, stdout phase parsing and
the stderr failure diagnostic must all survive the fix.

The real run() is exercised in a driver subprocess with a timeout so a
regression terminates instead of hanging the test suite; the child process
group is killed on timeout so nothing is left behind.
"""
import json
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Executed in its own process: imports the real tool and calls the real run().
# Reports either the measurements or the RuntimeError message as JSON.
DRIVER = textwrap.dedent("""\
    import json, sys
    from pathlib import Path
    sys.path.insert(0, sys.argv[1])
    from tools.profile_shared_json import run
    try:
        result = run(Path(sys.argv[2]), Path(sys.argv[3]), "deferred")
    except RuntimeError as exc:
        print(json.dumps({"error": str(exc)}))
    else:
        print(json.dumps({"measurements": result}))
""")

# More than a default 64 KiB pipe buffer, followed by valid stdout fields.
NOISY_OK = textwrap.dedent("""\
    #!/usr/bin/env python3
    import sys
    sys.stderr.write("E" * (1024 * 1024))
    sys.stderr.flush()
    print("phase_ms=1.5")
    print("total_ms=3.0")
    print("facts_ms=0.75")
""")

# Same overflowing stderr on the failure path, plus a marker to find in the
# RuntimeError diagnostic, then a nonzero exit.
NOISY_FAIL = textwrap.dedent("""\
    #!/usr/bin/env python3
    import sys
    sys.stderr.write("FATAL_DIAGNOSTIC_MARKER\\n" + "E" * (1024 * 1024))
    sys.stderr.flush()
    sys.exit(7)
""")


class VerboseChildTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.dir = Path(temporary.name)
        self.resources = self.dir / "res"
        self.resources.mkdir()
        self.env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")

    def fixture(self, name, source):
        path = self.dir / name
        path.write_text(source)
        path.chmod(0o755)
        return path

    def call_run(self, binary):
        proc = subprocess.Popen(
            [sys.executable, "-c", DRIVER, str(ROOT), str(binary), str(self.resources)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            start_new_session=True, env=self.env,
        )
        try:
            out, err = proc.communicate(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.communicate()
            self.fail("run() deadlocked: child stderr filled the pipe before wait4 returned")
        self.assertEqual(proc.returncode, 0, f"driver failed: {err[-2000:]}")
        return json.loads(out)

    @unittest.skipIf(not hasattr(os, "wait4"), "profiler uses POSIX wait4")
    def test_noisy_success_returns_measurements_without_deadlock(self):
        binary = self.fixture("noisy_ok.py", NOISY_OK)
        report = self.call_run(binary)
        self.assertNotIn("error", report)
        measurements = report["measurements"]
        self.assertEqual(measurements["phase_ms"], 1.5)
        self.assertEqual(measurements["total_ms"], 3.0)
        self.assertEqual(measurements["facts_ms"], 0.75)
        # wait4 per-child accounting is still present and plausible.
        self.assertGreaterEqual(measurements["cpu_ms"], 0.0)
        self.assertGreater(measurements["peak_rss_kib"], 0)

    @unittest.skipIf(not hasattr(os, "wait4"), "profiler uses POSIX wait4")
    def test_noisy_failure_keeps_stderr_in_the_error(self):
        binary = self.fixture("noisy_fail.py", NOISY_FAIL)
        report = self.call_run(binary)
        self.assertIn("error", report)
        self.assertIn("FATAL_DIAGNOSTIC_MARKER", report["error"])


if __name__ == "__main__":
    unittest.main()
