import json
import unittest

from tools.perf_bench import HISTOGRAM_EDGES_MS, OPTIONS, frames, rnd, summary


class PerfBenchStatsTests(unittest.TestCase):
    def test_lows_average_the_slowest_frames(self):
        times = [10.0] * 990 + [100.0] * 9 + [1000.0]
        s = frames("village", 0, times)
        self.assertEqual(s["frames"], 1000)
        self.assertEqual(s["avg_fps"], rnd(1000 * 1000 / (9900 + 900 + 1000)))
        # Slowest 1%: 9 frames of 100 ms and one of 1000 ms.
        self.assertEqual(s["low_1pct_fps"], rnd(1000 * 10 / 1900))
        # Slowest 0.1%: the single 1000 ms frame.
        self.assertEqual(s["low_01pct_fps"], 1.0)
        self.assertEqual((s["p50_ms"], s["p99_ms"], s["p999_ms"], s["max_ms"]), (10.0, 10.0, 100.0, 1000.0))
        self.assertEqual(sum(s["histogram"]), 1000)
        self.assertEqual(len(s["histogram"]), len(HISTOGRAM_EDGES_MS))
        self.assertEqual(s["histogram"][HISTOGRAM_EDGES_MS.index(10)], 990)
        self.assertEqual(s["histogram"][HISTOGRAM_EDGES_MS.index(100)], 9)
        self.assertEqual(s["histogram"][-1], 1, "1000 ms and over")

    def test_same_figures_as_horizon_bench(self):
        # crates/horizon-bench's frame_figures_and_histogram test, value for value.
        s = frames("flight", 0, [2.0] * 998 + [20.0, 1500.0])
        self.assertEqual((s["seconds"], s["low_1pct_fps"], s["low_01pct_fps"]), (3.516, 6.51, 0.667))
        self.assertEqual((s["histogram"][2], s["histogram"][13], s["histogram"][23]), (998, 1, 1))
        self.assertNotIn("cpu_p50_ms", s)
        r = summary([2.0, 4.0, 6.0])
        self.assertEqual(r, {"n": 3, "mean": 4.0, "stdev": 2.0, "min": 2.0, "max": 6.0, "ci95": 4.969})
        self.assertEqual(summary([5.0])["ci95"], None)
        self.assertIsNone(summary([None]))

    def test_a_short_flight_still_has_lows(self):
        s = frames("x", 0, [5.0, 20.0])
        self.assertEqual((s["low_1pct_fps"], s["low_01pct_fps"]), (50.0, 50.0))
        self.assertIsNone(frames("x", 0, []))
        json.dumps(s)

    def test_one_options_file_sets_the_render_distance(self):
        self.assertEqual(OPTIONS["renderDistance"].format(distance=16), "16")
        self.assertEqual(OPTIONS["graphicsPreset"], '"custom"')


if __name__ == "__main__":
    unittest.main()
