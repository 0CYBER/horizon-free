"""model_library_bench keeps every numeric metric the binaries actually printed."""
import unittest

from tools import model_library_bench as bench


class ParseRowTests(unittest.TestCase):
    def test_negative_and_exponent_values_are_real_samples(self):
        self.assertEqual(bench.parse_row("setup_ms=12.5 delta_ms=-3.0 rate=1e-3"),
                         {"setup_ms": 12.5, "delta_ms": -3.0, "rate": 0.001})

    def test_a_malformed_value_names_the_token_instead_of_raising_bare(self):
        with self.assertRaisesRegex(ValueError, "v='1.2.3'"):
            bench.parse_row("v=1.2.3")


class SummarizeTests(unittest.TestCase):
    def test_a_metric_missing_from_the_first_sample_is_still_reported(self):
        rows = [{"load_ms": 1.0}, *[{"load_ms": 100.0 + i, "alloc_peak_mb": 100.0 + i} for i in range(9)]]
        result = bench.summarize(rows)
        self.assertEqual(result["alloc_peak_mb"]["median"], 104.0)
        self.assertEqual(len(result["alloc_peak_mb"]["samples"]), 9)
        self.assertEqual(len(result["load_ms"]["samples"]), 10)

    def test_a_metric_absent_from_a_later_sample_does_not_abort(self):
        result = bench.summarize([{"a": 1.0, "b": 2.0}, {"a": 3.0}])
        self.assertEqual(result["b"]["samples"], [2.0])
