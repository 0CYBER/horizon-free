"""An empty Cargo --timings unit list is a parseable report and must still summarise."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("cargo_timings_empty", ROOT / "tools/ci/cargo_timings.py")
cargo_timings = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo_timings)


class CargoTimingsEmptyReportTests(unittest.TestCase):
    def test_empty_unit_data_returns_a_zero_count_summary(self):
        html = "const UNIT_DATA = [];\n"
        data = cargo_timings.units(html)
        self.assertEqual(data, [])
        self.assertEqual(cargo_timings.summary(data, 25), ["units=0 wall=0.0s summed=0.0s"])

    def test_empty_summary_keeps_the_normal_shape_for_nonempty_data(self):
        data = [{"name": "serde", "target": "", "mode": "todo", "start": 2.0, "duration": 3.0}]
        lines = cargo_timings.summary(data, 25)
        self.assertEqual(lines[0], "units=1 wall=5.0s summed=3.0s")
        self.assertEqual(len(lines), 2)


if __name__ == "__main__":
    unittest.main()
