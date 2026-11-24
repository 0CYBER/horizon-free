"""Regression for profile_shared_json executable-identity provenance.

The profiler must not pair sampled timings with a replacement binary's hash.
It freezes each executable's identity before sampling, rejects a persistent
change detected immediately before/after a run and before the report, and
writes the frozen (actually sampled) identity into the report.

The real CLI is exercised in bounded subprocesses with on-disk executable
fixtures: one fixture atomically replaces its own path mid-run, another is
stable. Expected hashes are computed independently with hashlib in this test.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "profile_shared_json.py"

# A: prints A's timings, then atomically replaces its own path with B's bytes.
# The printed timings are A's and must never be reported under B's hash.
REPLACING_FIXTURE = textwrap.dedent("""\
    #!/usr/bin/env python3
    import os
    import tempfile

    replacement = os.environ["REPLACEMENT_BINARY"]
    target = os.path.realpath(__file__)
    directory = os.path.dirname(target)
    fd, staged = tempfile.mkstemp(prefix=".profile-swap-", dir=directory)
    try:
        with os.fdopen(fd, "wb") as out:
            with open(replacement, "rb") as source:
                out.write(source.read())
        os.chmod(staged, 0o755)
        os.replace(staged, target)
    except BaseException:
        if os.path.exists(staged):
            os.unlink(staged)
        raise
    print("phase_ms=1")
    print("total_ms=2")
""")

REPLACEMENT_FIXTURE = textwrap.dedent("""\
    #!/usr/bin/env python3
    print("phase_ms=90")
    print("total_ms=100")
""")

STABLE_FIXTURE = textwrap.dedent("""\
    #!/usr/bin/env python3
    print("phase_ms=3")
    print("total_ms=6")
    print("facts_ms=0.5")
""")


@unittest.skipIf(not hasattr(os, "wait4"), "profiler uses POSIX wait4")
class ExecutableIdentityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.dir = Path(temporary.name).resolve()  # the report names the resolved path (macOS: /var is /private/var)
        self.resources = self.dir / "res"
        self.resources.mkdir()
        self.env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")

    def fixture(self, name, source):
        path = self.dir / name
        path.write_text(source)
        path.chmod(0o755)
        return path

    def call_cli(self, *args, env=None):
        return subprocess.run(
            [sys.executable, str(TOOL), "--resources", str(self.resources), *args],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            timeout=30, env=env or self.env,
        )

    def test_replacement_during_sample_fails_without_clobbering_old_output(self):
        replacement = self.fixture("replacement.py", REPLACEMENT_FIXTURE)
        a = self.fixture("a.py", REPLACING_FIXTURE)
        pre_run_hash = hashlib.sha256(a.read_bytes()).hexdigest()
        replacement_hash = hashlib.sha256(replacement.read_bytes()).hexdigest()
        self.assertNotEqual(pre_run_hash, replacement_hash)

        output = self.dir / "report.json"
        sentinel = '{"old": "report must survive a failed run"}\n'
        output.write_text(sentinel)

        env = dict(self.env, REPLACEMENT_BINARY=str(replacement))
        proc = self.call_cli("--binary", str(a), "--repeats", "1",
                             "--variant", f"fix={a}@deferred",
                             "--output", str(output), env=env)

        self.assertNotEqual(
            proc.returncode, 0,
            f"A replaced itself mid-run but the profiler still succeeded: {proc.stdout[-1000:]}")
        self.assertIn("identity", proc.stderr)
        self.assertIn("sha256", proc.stderr)
        # The old report is untouched: the failure happened before any write.
        self.assertEqual(output.read_text(), sentinel)
        # Independently confirm the path now holds B (the fixture did replace it).
        self.assertEqual(hashlib.sha256(a.read_bytes()).hexdigest(), replacement_hash)

    def test_stable_binary_reports_independently_computed_sha_for_alias_modes(self):
        stable = self.fixture("stable.py", STABLE_FIXTURE)
        expected = hashlib.sha256(stable.read_bytes()).hexdigest()
        output = self.dir / "stable_report.json"

        # Default mode set with one shared executable path exercises same-binary
        # comparison and alias paths: all four modes pin the same identity.
        proc = self.call_cli("--binary", str(stable), "--repeats", "1", "--output", str(output))
        self.assertEqual(proc.returncode, 0, f"stable run failed: {proc.stderr[-2000:]}")
        report = json.loads(output.read_text())
        for mode in ("baseline", "pr607", "shared", "deferred"):
            self.assertEqual(report["executables"][mode]["path"], str(stable))
            self.assertEqual(report["executables"][mode]["sha256"], expected)
        # Timings are the stable fixture's, matching the pinned identity.
        self.assertEqual(report["metrics"]["deferred"]["phase_ms"]["median"], 3.0)


if __name__ == "__main__":
    unittest.main()
