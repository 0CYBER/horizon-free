"""The Cargo timings summary reads the report Cargo writes and ranks units by duration."""
import importlib.util
import json
from pathlib import Path
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


if __name__ == "__main__":
    unittest.main()
