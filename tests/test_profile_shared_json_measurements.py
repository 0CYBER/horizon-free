"""Admission regression for profile_shared_json measurements.

A standalone executable fixture prints child-reported measurement lines; the
real profiler CLI runs it through --variant. Invalid samples (NaN, infinity,
negative, malformed, missing required) must exit nonzero with a diagnostic that
names the binary, mode and field, and must leave an existing report untouched.
Valid zero/finite samples must produce a report a strict JSON decoder accepts.
No game, game data or network is involved.
"""
import json
import math
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "profile_shared_json.py"

FIXTURE = f"""#!{sys.executable}
import os, sys
sys.stdout.write(os.environ.get("PROFILE_FIXTURE_LINES", ""))
"""

SENTINEL = "SENTINEL_REPORT_MUST_NOT_CHANGE\n"


def strict_loads(raw):
    """Decode JSON, rejecting nonstandard NaN/Infinity tokens outright."""
    def reject_constant(token):
        raise AssertionError(f"nonstandard JSON token {token!r} in report")

    def finite_float(text):
        value = float(text)
        if not math.isfinite(value):
            raise AssertionError(f"non-finite number {text!r} in report")
        return value

    return json.loads(raw, parse_constant=reject_constant, parse_float=finite_float)


def all_finite_nonnegative(value):
    if isinstance(value, bool):
        return True
    if isinstance(value, (int, float)):
        return math.isfinite(value) and value >= 0
    if isinstance(value, list):
        return all(all_finite_nonnegative(item) for item in value)
    if isinstance(value, dict):
        return all(all_finite_nonnegative(item) for item in value.values())
    return True


class MeasurementAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.dir = Path(temporary.name)
        self.resources = self.dir / "res"
        self.resources.mkdir()
        self.binary = self.dir / "fixture.py"
        self.binary.write_text(FIXTURE)
        self.binary.chmod(0o755)
        self.report = self.dir / "report.json"
        self.env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", TMPDIR="/tmp/opencode")

    def profile(self, lines):
        env = dict(self.env, PROFILE_FIXTURE_LINES=lines)
        return subprocess.run(
            [sys.executable, str(TOOL), "--resources", str(self.resources),
             "--repeats", "1", "--variant", f"fix={self.binary}@deferred",
             "--output", str(self.report)],
            capture_output=True, text=True, env=env, timeout=30,
        )

    def runtime_error_line(self, proc):
        diagnostics = [line for line in proc.stderr.splitlines()
                       if line.startswith("RuntimeError:")]
        self.assertTrue(diagnostics, f"no RuntimeError diagnostic: {proc.stderr[-1500:]}")
        return diagnostics[-1]

    def test_invalid_measurement_matrix_is_rejected_without_touching_report(self):
        cases = [
            ("phase_ms=nan\ntotal_ms=2.0\n", "phase_ms", "not finite"),
            ("phase_ms=inf\ntotal_ms=2.0\n", "phase_ms", "not finite"),
            ("phase_ms=-1\ntotal_ms=2.0\n", "phase_ms", "negative"),
            ("phase_ms=abc\ntotal_ms=2.0\n", "phase_ms", "not numeric"),
            ("phase_ms=1=2\ntotal_ms=2.0\n", "phase_ms", "not numeric"),
            ("phase_ms=1.5\ntotal_ms=nan\n", "total_ms", "not finite"),
            ("phase_ms=1.5\ntotal_ms=inf\n", "total_ms", "not finite"),
            ("phase_ms=1.5\ntotal_ms=-0.25\n", "total_ms", "negative"),
            ("phase_ms=1.5\ntotal_ms=2.0\nfacts_ms=nan\n", "facts_ms", "not finite"),
            ("phase_ms=1.5\ntotal_ms=2.0\nfacts_ms=inf\n", "facts_ms", "not finite"),
            ("phase_ms=1.5\ntotal_ms=2.0\nfacts_ms=-5\n", "facts_ms", "negative"),
            ("phase_ms=1.5\ntotal_ms=2.0\nbehaviour_json_ms=-2.5\n",
             "behaviour_json_ms", "negative"),
            ("phase_ms=1.5\ntotal_ms=2.0\npost_rss_kib=-1\n", "post_rss_kib", "negative"),
            ("total_ms=2.0\n", "phase_ms", "missing"),
        ]
        for lines, field, reason in cases:
            with self.subTest(lines=lines):
                self.report.write_text(SENTINEL)
                proc = self.profile(lines)
                self.assertNotEqual(proc.returncode, 0,
                                    f"invalid measurement accepted: {lines!r}")
                diagnostic = self.runtime_error_line(proc)
                self.assertIn(field, diagnostic)
                self.assertIn("deferred", diagnostic)
                self.assertIn(str(self.binary), diagnostic)
                self.assertIn(reason, diagnostic)
                self.assertEqual(self.report.read_text(), SENTINEL)
                self.assertNotIn('"scope"', proc.stdout)

    def test_valid_zero_and_finite_samples_produce_strict_json(self):
        controls = [
            ("phase_ms=0\ntotal_ms=0\nfacts_ms=0\npost_rss_kib=0\n",
             {"phase_ms": 0.0, "total_ms": 0.0, "facts_ms": 0.0, "post_rss_kib": 0.0}),
            ("phase_ms=1.5\ntotal_ms=3.25\nbehaviour_json_ms=0.125\n",
             {"phase_ms": 1.5, "total_ms": 3.25, "behaviour_json_ms": 0.125}),
        ]
        for lines, expected in controls:
            with self.subTest(lines=lines):
                self.report.write_text(SENTINEL)
                proc = self.profile(lines)
                self.assertEqual(proc.returncode, 0,
                                 f"valid measurement rejected: {proc.stderr[-1500:]}")
                printed = strict_loads(proc.stdout)
                saved = strict_loads(self.report.read_text())
                self.assertEqual(printed, saved)
                self.assertTrue(all_finite_nonnegative(saved))
                metrics = saved["metrics"]["fix"]
                self.assertEqual(set(metrics),
                                 set(expected) | {"cpu_ms", "peak_rss_kib"})
                for field, value in expected.items():
                    stat = metrics[field]
                    self.assertEqual(stat["samples"], [value])
                    self.assertEqual(stat["median"], value)
                    self.assertEqual(stat["min"], value)
                    self.assertEqual(stat["max"], value)


if __name__ == "__main__":
    unittest.main()
