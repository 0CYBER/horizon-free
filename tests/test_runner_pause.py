"""tools/ci/runner_pause.py: the Linux CI runner works only while nobody is at the screen or playing."""
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools.ci import runner_pause


class Machine:
    """A fake desktop: a clock, who is there, and the runner service. The watcher samples every 15 s."""

    def __init__(self, signs):
        self.signs = signs  # (from, until, reason): a sign of someone at the screen or playing
        self.clock = 0.0
        self.active = False
        self.job_until = None  # the runner is in a job until then
        self.throttles = []  # (clock, on)
        self.calls = []

    def reason(self, hot, keys):
        return next((reason for start, end, reason in self.signs if start <= self.clock < end), None)

    def systemctl(self, *arguments):
        self.calls.append((self.clock, arguments))
        if arguments[1:] == (runner_pause.SERVICE,):
            self.active = {"start": True, "stop": False}.get(arguments[0], self.active)
        return mock.Mock(returncode=0 if arguments[:2] != ("is-active", "--quiet") or self.active else 3)

    def busy(self):
        return self.job_until is not None and self.clock < self.job_until

    def sleep(self, seconds):
        self.clock += seconds
        if self.clock > 2000:
            raise StopIteration

    def run(self):
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.multiple(runner_pause, MANUAL=Path(directory, "pause"), STATE=Path(directory, "paused"),
                                    LOG=Path(directory, "pause.log"), INTERVAL=15, RESUME_AFTER=300,
                                    pause_reason=self.reason, systemctl=self.systemctl, runner_busy=self.busy,
                                    throttle=lambda on: self.throttles.append((self.clock, on))), \
                mock.patch.object(runner_pause.time, "monotonic", lambda: self.clock), \
                mock.patch.object(runner_pause.time, "sleep", self.sleep):
            try:
                runner_pause.main()
            except StopIteration:  # the clock ran out
                pass

    def first(self, verb, after=0):
        return next(clock for clock, arguments in self.calls
                    if arguments == (verb, runner_pause.SERVICE) and clock >= after)


class RunnerPauseTests(unittest.TestCase):
    def test_starts_only_after_nobody_was_there_for_resume_after(self):
        machine = Machine([(0, 600, "session 3 of ludovic at the screen (seat0)")])
        machine.run()
        self.assertEqual(machine.first("start"), 585 + 300)  # the last sample that saw him

    def test_a_game_stops_an_idle_runner_at_once(self):
        machine = Machine([(1000, 1300, "process 42: wine64-preloader game.exe")])
        machine.run()
        self.assertEqual(machine.first("start"), 300)
        self.assertEqual(machine.first("stop", after=300), 1005)
        self.assertEqual(machine.first("start", after=1005), 1290 + 300)

    def test_a_job_under_way_is_throttled_then_stopped_when_it_ends(self):
        machine = Machine([(1000, 1300, "GPU busy: process 7 (game)")])
        machine.job_until = 1200
        machine.run()
        self.assertIn((1005, True), machine.throttles)  # the game started: the job slows down at once,
        self.assertIn((1005, ("stop", "horizon-parity-worker-*.scope")), machine.calls)  # other machines' work goes
        stop = machine.first("stop", after=300)
        self.assertGreaterEqual(stop, 1200)  # and the runner stops once it ends, not before
        self.assertLess(stop, 1215)
        self.assertEqual(machine.throttles[-1], (1290 + 300, False))

    def test_the_desktop_is_free_when_locked_or_untouched_for_15_minutes(self):
        cases = [  # (session, seconds since the last input, someone there)
            (("3", False), 30, True), (("3", True), 30, True),  # any input pauses at once, even on the lock screen
            (("3", False), 120, True), (("3", False), 1000, False),  # unlocked: free after 15 min untouched
            (("3", True), 120, False), (None, 120, False),  # locked, or nobody logged in
        ]
        for session, idle, there in cases:
            with self.subTest(session=session, idle=idle), \
                    mock.patch.object(runner_pause, "seat_session", lambda: session):
                self.assertEqual(runner_pause.at_the_screen(idle) is not None, there)


if __name__ == "__main__":
    unittest.main()
