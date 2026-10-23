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

    def test_window_sizes_name_their_reports(self):
        from tools.perf_bench import SIZES, parse_sizes, size_label
        self.assertEqual(parse_sizes("1280x720,1920X1080"), [(1280, 720), (1920, 1080)])
        self.assertEqual(SIZES, [(1280, 720), (1920, 1080)])
        self.assertEqual(size_label("horizon-m4", 1280, 720), "horizon-m4")
        self.assertEqual(size_label("horizon-m4", 1920, 1080), "horizon-m4-1080p")
        self.assertEqual(size_label("horizon-m4", 2560, 1080), "horizon-m4-2560x1080")

    def test_one_options_file_sets_the_render_distance(self):
        self.assertEqual(OPTIONS["renderDistance"].format(distance=16), "16")
        self.assertEqual(OPTIONS["graphicsPreset"], '"custom"')


if __name__ == "__main__":
    unittest.main()


class WaitLoadedTests(unittest.TestCase):
    """wait_loaded on a scripted game with a fake clock: one status per 50 ms poll."""

    def run_script(self, statuses, min_chunks=0):
        import tools.perf_bench as pb
        clock = [0.0]
        it = iter(statuses)

        def control(_command, timeout=None):
            return next(it)

        real_time, real_sleep = pb.time.time, pb.time.sleep
        pb.time.time = lambda: clock[0]
        pb.time.sleep = lambda s: clock.__setitem__(0, clock[0] + s)
        try:
            return pb.wait_loaded(control, lambda p: True, 60, min_chunks)
        finally:
            pb.time.time, pb.time.sleep = real_time, real_sleep

    @staticmethod
    def status(chunks, meshed, stages=None):
        s = {"player": {"position": [0, 0, 0]}, "pending_chunks": 0, "screen": "game",
             "chunks": chunks, "generated_chunks": chunks, "meshed_sections": meshed}
        if stages is not None:
            s["streaming"] = {"queued": 0, "generating": 0, "protos": 296, "decorating": 0,
                              "finishing": 0, "lighting": 0, "meshing": stages}
        return s

    def test_stages_end_the_load_before_the_remesh_tail(self):
        # 20 polls (1 s) of meshing work, then idle stages while blocks keep remeshing
        # at 400 meshes/s, with a short burst of work every 2 s.
        loading = [self.status(900, 1000 * i, stages=500) for i in range(20)]
        tail = [self.status(901, 20000 + 20 * i, stages=40 if i % 40 == 0 else 0) for i in range(400)]
        self.assertAlmostEqual(self.run_script(loading + tail), 1.0, places=6)

    def test_old_builds_wait_for_meshing_to_go_quiet(self):
        busy = [self.status(901, 1000 * i) for i in range(20)]
        quiet = [self.status(901, 20000 + i) for i in range(40)]
        self.assertAlmostEqual(self.run_script(busy + quiet), 1.0, places=6)

    def test_a_quiet_start_is_not_a_load(self):
        # Right after a teleport Horizon reports steady counts and nothing queued for 1.5 s before it loads:
        # with a target the clock runs on until the chunks are there and the meshing has gone quiet.
        idle = [self.status(3, 0, stages=0) for _ in range(30)]
        loading = [self.status(30 * (i + 1), 500 * i, stages=500) for i in range(20)]
        tail = [self.status(600, 10000, stages=0) for _ in range(60)]
        took = self.run_script(idle + loading + tail, min_chunks=500)
        self.assertGreater(took, 2.4)
        self.assertLess(took, 4.0)
        # Without one (the old call), the quiet start ends the wait at once.
        self.assertLess(self.run_script(idle + loading + tail), 1.6)
