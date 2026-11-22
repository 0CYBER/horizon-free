"""The Cargo timings summary reads the report Cargo writes and ranks units by duration."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("cargo_timings", ROOT / "tools/ci/cargo_timings.py")
cargo_timings = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo_timings)


class CargoTimingsTests(unittest.TestCase):
    def test_ranks_units_by_duration_and_reports_the_build_wall(self):
        data = [
            {"name": "horizon-client", "target": " (test)", "mode": "todo", "start": 10.0, "duration": 300.0},
            {"name": "serde", "target": "", "mode": "todo", "start": 0.0, "duration": 4.0},
            {"name": "horizon-world", "target": " integration (test)", "mode": "todo", "start": 5.0, "duration": 40.0},
        ]
        html = f"<script>\nconst UNIT_DATA = {json.dumps(data)};\nconst CONCURRENCY_DATA = [];\n</script>"
        lines = cargo_timings.summary(cargo_timings.units(html), top=2)
        self.assertEqual(lines[0], "units=3 wall=310.0s summed=344.0s")
        self.assertIn("horizon-client (test)", lines[1])
        self.assertIn("horizon-world integration (test)", lines[2])
        self.assertEqual(len(lines), 3)

    def test_a_report_without_unit_data_is_refused(self):
        with self.assertRaisesRegex(ValueError, "no UNIT_DATA"):
            cargo_timings.units("<html></html>")

    def test_a_negative_top_is_refused_instead_of_dropping_the_slowest_units(self):
        data = [
            {"start": 0.0, "duration": 10.0, "name": "slow", "target": "lib", "mode": "build"},
            {"start": 10.0, "duration": 1.0, "name": "fast", "target": "lib", "mode": "build"},
        ]
        with self.assertRaisesRegex(ValueError, "--top must be non-negative"):
            cargo_timings.summary(data, -1)
        self.assertEqual(
            cargo_timings.summary(data, 25),
            [
                "units=2 wall=11.0s summed=11.0s",
                "    10.0s start=    0.0s slow lib (build)",
                "     1.0s start=   10.0s fast lib (build)",
            ],
        )

    def test_the_command_line_rejects_a_negative_top_with_an_argument_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "cargo-timing.html"
            report.write_text("const UNIT_DATA = [];\n")
            result = subprocess.run(
                [sys.executable, str(ROOT / "tools/ci/cargo_timings.py"), str(report), "--top", "-1"],
                text=True, capture_output=True,
            )
        self.assertEqual(result.returncode, 2)
        self.assertIn("--top must be non-negative", result.stderr)


if __name__ == "__main__":
    unittest.main()
