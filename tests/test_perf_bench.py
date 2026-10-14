import unittest

from tools.perf_bench import OPTIONS, stats


class PerfBenchStatsTests(unittest.TestCase):
    def test_lows_average_the_slowest_frames(self):
        times = [10.0] * 990 + [100.0] * 9 + [1000.0]
        s = stats(times)
        self.assertEqual(s["frames"], 1000)
        self.assertAlmostEqual(s["avg_fps"], 1000 * 1000 / (9900 + 900 + 1000))
        # Slowest 1%: 9 frames of 100 ms and one of 1000 ms.
        self.assertAlmostEqual(s["low_1pct_fps"], 1000 * 10 / 1900)
        # Slowest 0.1%: the single 1000 ms frame.
        self.assertAlmostEqual(s["low_01pct_fps"], 1.0)
        self.assertEqual(s["p50_ms"], 10.0)
        self.assertEqual(s["p99_ms"], 10.0)
        self.assertEqual(s["p999_ms"], 100.0)
        self.assertEqual(s["max_ms"], 1000.0)
        self.assertAlmostEqual(sum(s["histogram"].values()), 1.0)
        self.assertAlmostEqual(s["histogram"][">=100ms"], 0.01)

    def test_a_short_flight_still_has_lows(self):
        s = stats([5.0, 20.0])
        self.assertAlmostEqual(s["low_1pct_fps"], 50.0)
        self.assertAlmostEqual(s["low_01pct_fps"], 50.0)
        self.assertEqual(stats([]), {"frames": 0})

    def test_one_options_file_sets_the_render_distance(self):
        self.assertEqual(OPTIONS["renderDistance"].format(distance=16), "16")
        self.assertEqual(OPTIONS["graphicsPreset"], '"custom"')


if __name__ == "__main__":
    unittest.main()
