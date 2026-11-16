"""A test that dies without its own panic still says why: the runtime's abort and other threads' panics."""
import contextlib
import io
import unittest

from tools.ci import ignored_tests as ci


def render(output):
    with contextlib.redirect_stdout(io.StringIO()) as log:
        ci.failure_diagnostics(output)
    return log.getvalue()


class AbortContext(unittest.TestCase):
    def test_a_stack_overflow_is_named(self):
        log = render("test game::capture ... FAILED\n"
                     "\n"
                     "thread 'game::capture' has overflowed its stack\n"
                     "fatal runtime error: stack overflow, aborting\n")
        self.assertTrue(log.startswith("FATAL: process aborted (signal 6: stack overflow) on test game::capture\n"
                                       "ordinary failures (the process kept running after them):\n"
                                       "  failed test: game::capture\n"), log)

    def test_a_server_threads_panic_and_the_abort_after_it_are_shown_without_payload(self):
        log = render("test game::capture ... FAILED\n"
                     "thread 'Server thread' panicked at private.rs:1:1:\n"
                     "PRIVATE world name SecretWorld\n"
                     "thread 'Server thread' panicked while processing panic. aborting.\n")
        self.assertIn("FATAL: process aborted (signal 6: a panic while panicking) on thread 'Server thread'\n", log)
        self.assertIn("  failed test: game::capture\n"
                      "  on a non-test thread:\n"
                      "  panic location withheld (not a bounded tracked Rust location)\n"
                      "  panic reason: custom panic; payload withheld (resource privacy)\n", log)
        self.assertNotIn("SecretWorld", log)

    def test_nothing_known_stays_missing(self):
        self.assertIn("panic context missing from captured output",
                      render("test game::capture ... FAILED\nsomething else\n"))


class ProcessLevel(unittest.TestCase):
    def test_a_crash_with_no_failed_test_still_names_the_abort_and_the_panicking_thread(self):
        log = render("test game::a ... ok\n"
                     "thread 'game::capture' has overflowed its stack\n"
                     "fatal runtime error: stack overflow, aborting\n"
                     "thread 'Server thread' panicked at private.rs:1:1:\n"
                     "PRIVATE text\n")
        self.assertTrue(log.startswith("FATAL: process aborted (signal 6: stack overflow) on test game::capture\n"), log)
        self.assertIn("  no failed test named; panics seen:\n  panic on thread 'Server thread':\n", log)
        self.assertNotIn("ordinary failures", log)
        self.assertNotIn("PRIVATE", log)

    def test_an_odd_thread_name_is_withheld(self):
        log = render("thread 'x\x1b[31m' has overflowed its stack\n")
        self.assertIn("(signal 6: stack overflow) on a thread (name withheld)", log)

    def test_the_fatal_thread_comes_first_and_ordinary_failures_after(self):
        # The case that misled a reader: an ordinary failure printed before another test's overflow.
        log = render("test game::conversion ... FAILED\n"
                     "thread 'game::conversion' panicked at private.rs:1:1:\n"
                     "assertion failed: x\n"
                     "thread 'game::mounts::feeding' has overflowed its stack\n"
                     "fatal runtime error: stack overflow, aborting\n")
        self.assertTrue(log.startswith("FATAL: process aborted (signal 6: stack overflow) on test game::mounts::feeding\n"
                                       "ordinary failures (the process kept running after them):\n"
                                       "  failed test: game::conversion\n"), log)
        self.assertNotIn("process aborted", log.split("ordinary failures", 1)[1])

    def test_an_abort_without_a_thread_says_the_fatal_thread_is_unknown(self):
        log = render("test game::a ... ok\nmemory allocation of 99 bytes failed\n")
        self.assertTrue(log.startswith("FATAL: process aborted (signal 6: a failed memory allocation); fatal thread "
                                       "unknown, see the last started tests below\n"), log)

    def test_no_abort_prints_no_fatal_line(self):
        log = render("test game::a ... FAILED\nthread 'game::a' panicked at private.rs:1:1:\nassertion failed: x\n")
        self.assertNotIn("FATAL", log)
        self.assertNotIn("ordinary failures", log)
        self.assertIn("  failed test: game::a\n", log)

    def test_the_crash_report_names_long_runners_and_unreported_tests(self):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            ci.crash_report("pkg", "test game::slow has been running for over 60 seconds\ntest game::a ... ok\n",
                            {"game::a", "game::slow", "game::never"}, {"game::a": True})
        out = log.getvalue()
        self.assertIn("pkg: last started tests, still running past 60 s when it crashed: game::slow\n", out)
        self.assertIn("pkg: 2 listed tests never reported (started or not yet started): game::never, game::slow\n", out)


if __name__ == "__main__":
    unittest.main()
