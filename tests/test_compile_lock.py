"""tools/compile_lock.py: jobs on the self-hosted Mac take turns at compiling (#19151). Real processes and flock
where the system has them (Linux CI), a temporary folder."""

import ast
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from unittest import mock

from tools import compile_lock
from tools.compile_lock import Turn, WAIT

ROOT = Path(__file__).resolve().parents[1]
try:
    import fcntl  # noqa: F401
    HAS_FLOCK = True
except ImportError:
    HAS_FLOCK = False
needs_flock = unittest.skipUnless(HAS_FLOCK and os.name == "posix", "flock and process groups (POSIX)")
MAC = {"RUNNER_ENVIRONMENT": "self-hosted", "GITHUB_RUN_ID": "77", "GITHUB_WORKFLOW": "parity",
       "GITHUB_JOB": "parity", "RUNNER_NAME": "mac-runner"}


def process_gone(pid, seconds=5.0):
    """Whether the process has exited within `seconds`: kill(pid, 0) fails, or /proc (where there is one) shows a
    zombie nobody reaped yet. A missing /proc never counts as gone."""
    deadline = time.monotonic() + seconds
    while True:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        try:
            with open(f"/proc/{pid}/stat") as handle:
                if handle.read().rsplit(")", 1)[1].split()[0] == "Z":
                    return True
        except OSError:
            pass
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.1)


class Clock:
    """Fake monotonic time: a waiter's sleep moves it."""

    def __init__(self):
        self.now, self.slept = 100.0, 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        self.slept += seconds


HOLDER = textwrap.dedent("""
    import sys, time
    sys.path.insert(0, sys.argv[1])
    from tools.compile_lock import Turn
    turn = Turn(sys.argv[2], folder=sys.argv[3], say=lambda line: None)
    turn.__enter__()
    print("held" if turn.held else "not held", flush=True)
    time.sleep(120)
""")


class CompileLockTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = os.path.join(self.temp.name, "lock")
        self.clock = Clock()
        self.lines = []

    def turn(self, role, job=None, sleep=None, folder=None, environ=None, platform="darwin"):
        environ = dict(MAC if environ is None else environ, **({"GITHUB_JOB": job} if job else {}))
        return Turn(role, folder=folder or self.folder, environ=environ, platform=platform, clock=self.clock,
                    sleep=sleep or self.clock.sleep, say=self.lines.append)

    def holder_process(self, role="pr", environ=None):
        """A job in another process holding (or trying for) the turn; ended at cleanup."""
        process = subprocess.Popen([sys.executable, "-c", HOLDER, str(ROOT), role, self.folder],
                                   stdout=subprocess.PIPE, text=True, env=dict(os.environ, **(environ or MAC)))
        self.addCleanup(lambda: (process.poll() is None and process.kill(), process.wait(),
                                 process.stdout.close()))
        self.assertEqual(process.stdout.readline().strip(), "held")
        return process

    # The turn

    @needs_flock
    def test_taken_and_released(self):
        turn = self.turn("pr")
        self.assertTrue(turn.acquire())
        self.assertIn("parity / parity - role pr run 77 runner mac-runner pid ", Path(self.folder, "holder").read_text())
        self.clock.now += 90
        turn.release()
        self.assertEqual(self.lines, ["  compile lock: taken, no wait", "  compile lock: released after 1.5 min"])
        self.assertEqual(Path(self.folder, "holder").read_text(), "")
        self.assertEqual(sorted(os.listdir(self.folder)), ["holder", "priority", "turn"])
        self.assertTrue(self.turn("pr").acquire(), "free again")

    @needs_flock
    def test_a_killed_holder_frees_the_turn_at_once(self):
        process = self.holder_process("main")
        self.assertFalse(self.turn("build", sleep=lambda seconds: self.clock.sleep(WAIT["build"])).acquire())
        process.send_signal(signal.SIGKILL)
        process.wait()
        self.clock.slept = 0
        self.assertTrue(self.turn("build").acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_a_bounded_wait_goes_ahead_and_names_the_holder(self):
        self.holder_process("main", dict(MAC, GITHUB_WORKFLOW="parity-main", GITHUB_JOB="parity-main"))
        waiter = self.turn("build", job="build macos")
        self.assertFalse(waiter.acquire())
        self.assertEqual(self.clock.slept, WAIT["build"])
        self.assertTrue(self.lines[0].startswith("  compile lock: waiting: parity-main / parity-main - role main run "
                                                 "77 runner mac-runner pid "))
        self.assertTrue(self.lines[-1].startswith("  compile lock: waited 60 min, this job's limit; compiling "
                                                  "without the turn while parity-main / parity-main - role main"))
        self.assertEqual(len(self.lines), 2)

    @needs_flock
    def test_the_lock_never_reaches_a_child_process(self):
        # A child that inherits every descriptor it may (close_fds=False) and outlives the release, as sccache's
        # server or a lingering cargo would: the turn is free all the same.
        turn = self.turn("main")
        self.assertTrue(turn.acquire())
        self.addCleanup(turn.release)
        child = subprocess.Popen([sys.executable, "-c", "import time; print('up', flush=True); time.sleep(60)"],
                                 close_fds=False, stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (child.kill(), child.wait(), child.stdout.close()))
        # Popen can return before the kernel has closed the child's close-on-exec descriptors, and until then the
        # child's copies keep the locks. The child's own first line says its exec is over.
        self.assertTrue(select.select([child.stdout], [], [], 60)[0], "the child never started")
        self.assertEqual(child.stdout.readline().strip(), "up")
        turn.release()
        self.assertIsNone(child.poll())
        self.assertTrue(self.turn("pr").acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_the_wrappers_command_never_holds_the_turn(self):
        command = ("import subprocess, sys; subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], "
                   "start_new_session=True, close_fds=False, stdout=subprocess.DEVNULL)")
        wrapper = subprocess.run([sys.executable, str(ROOT / "tools/compile_lock.py"), "build", "--", sys.executable,
                                  "-c", command], env=dict(os.environ, HORIZON_COMPILE_LOCK_DIR=self.folder),
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, timeout=60)
        self.assertEqual(wrapper.returncode, 0)
        self.assertIn("compile lock: taken, no wait", wrapper.stdout)
        self.assertTrue(self.turn("pr").acquire(), "the grandchild still runs, and the turn is free")
        self.assertEqual(self.clock.slept, 0)

    # Priority

    @needs_flock
    def test_a_pull_request_waits_while_a_priority_job_is_here_even_with_the_turn_free(self):
        main = self.turn("main", job="parity-main")
        main.open()
        self.assertTrue(main.arrive())  # arrived; the turn itself is free
        pr = self.turn("pr")
        self.assertFalse(pr.acquire())
        self.assertEqual(self.clock.slept, WAIT["pr"])
        self.assertIn("jobs ahead of pull requests (parity-main, a Mac build) are here", self.lines[0])
        self.assertTrue(main.try_turn(), "the free turn was left to parity-main")
        main.release()
        self.clock.slept = 0
        self.assertTrue(self.turn("pr").acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_a_priority_job_arriving_between_a_pull_requests_checks(self):
        # Hold `priority` exclusively (the pull request's step 1); the priority job's arrival is refused, so it is
        # not here yet: the pull request takes the turn, and the priority job then waits for the turn.
        pr, main = self.turn("pr"), self.turn("main", job="parity-main")
        pr.open()
        main.open()
        arrivals = []
        real = pr.lock

        def lock(descriptor, mode):
            taken = real(descriptor, mode)
            if descriptor == pr.priority and mode == pr.fcntl.LOCK_EX and taken:
                arrivals.append(main.arrive())  # between step 1 and step 2
            return taken
        pr.lock = lock
        self.assertTrue(pr.try_turn())
        self.assertEqual(arrivals, [False])
        self.assertTrue(main.arrive(), "step 3 let `priority` go")
        self.assertFalse(main.try_turn(), "it waits for the turn like anybody")
        # And the other order: arrived first, the pull request's step 1 is refused.
        pr.release()
        other = self.turn("pr")
        other.open()
        self.assertFalse(other.try_turn())
        self.assertTrue(other.ahead)
        self.assertTrue(main.try_turn())

    @needs_flock
    def test_priority_jobs_wait_for_no_pull_request_that_only_waits(self):
        waiting = self.turn("pr")
        waiting.open()  # a pull request in its wait loop holds no lock between attempts
        main, build = self.turn("main", job="parity-main"), self.turn("build", job="build macos")
        self.assertTrue(main.acquire())
        main.release()
        self.assertTrue(build.acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_a_priority_job_that_goes_ahead_keeps_pull_requests_waiting(self):
        self.holder_process("pr")
        main = self.turn("main", job="parity-main")
        self.assertFalse(main.acquire())
        self.assertEqual(self.clock.slept, WAIT["main"])
        self.clock.slept = 0
        late = self.turn("pr", job="late")
        self.assertFalse(late.acquire())
        self.assertIn("jobs ahead of pull requests", self.lines[-1])
        main.release()

    # Fail open, and what the folder may hold

    @needs_flock
    def test_fails_open_where_the_folder_cannot_be_made(self):
        blocker = Path(self.temp.name, "a-file")
        blocker.write_text("")
        with self.turn("main", folder=str(blocker / "lock")) as turn:
            self.assertFalse(turn.held)
        self.assertEqual(len(self.lines), 1)
        self.assertTrue(self.lines[0].endswith("; compiling without it"))
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_unexpected_folder_entries_never_block_or_reach_outside(self):
        outside = Path(self.temp.name, "outside")
        outside.mkdir()
        target = outside / "precious"
        target.write_text("keep")
        cases = {
            "symlink root": lambda: os.symlink(outside, self.folder),
            "symlink turn": lambda: (os.mkdir(self.folder), os.symlink(target, os.path.join(self.folder, "turn"))),
            "fifo turn": lambda: (os.mkdir(self.folder), os.mkfifo(os.path.join(self.folder, "turn"))),
            "fifo priority": lambda: (os.mkdir(self.folder), os.mkfifo(os.path.join(self.folder, "priority"))),
            "directory turn": lambda: (os.mkdir(self.folder), os.mkdir(os.path.join(self.folder, "turn"))),
            "hard link turn": lambda: (os.mkdir(self.folder), os.link(target, os.path.join(self.folder, "turn"))),
        }
        for name, make in cases.items():
            with self.subTest(name):
                self.reset_folder()
                make()
                self.lines.clear()
                started = time.monotonic()
                with self.turn("pr") as turn:
                    self.assertFalse(turn.held)
                self.assertLess(time.monotonic() - started, 5)
                self.assertEqual(len(self.lines), 1, self.lines)
                self.assertTrue(self.lines[0].endswith("compiling without it"))
                self.assertEqual(target.read_text(), "keep")
                self.assertEqual(sorted(os.listdir(outside)), ["precious"])
        for name, make in {
            "symlink holder": lambda: os.symlink(target, os.path.join(self.folder, "holder")),
            "hard link holder": lambda: os.link(target, os.path.join(self.folder, "holder")),
            "fifo holder": lambda: os.mkfifo(os.path.join(self.folder, "holder")),
            "directory holder": lambda: os.mkdir(os.path.join(self.folder, "holder")),
        }.items():
            with self.subTest(name):
                self.reset_folder()
                os.mkdir(self.folder)
                make()
                started = time.monotonic()
                with self.turn("pr") as turn:
                    self.assertTrue(turn.held, "the display file never decides")
                    self.assertEqual(turn.holder(), "")
                self.assertLess(time.monotonic() - started, 5)
                self.assertEqual(target.read_text(), "keep")

    @needs_flock
    def test_a_folder_and_lock_files_the_job_cannot_replace(self):
        # Made beforehand (by another user, say): the folder 0555, the lock files 0666. The job opens them as they
        # are; it can't create or replace anything there, and needs not.
        os.mkdir(self.folder)
        for name in ("turn", "priority"):
            Path(self.folder, name).write_bytes(b"")
            os.chmod(os.path.join(self.folder, name), 0o666)
        os.chmod(self.folder, 0o555)
        self.addCleanup(os.chmod, self.folder, 0o755)
        with self.turn("pr") as turn:
            self.assertTrue(turn.held)
            self.assertEqual(self.turn("main").holder(), "", "no `holder` could be made: nothing to show")
        self.assertEqual(sorted(os.listdir(self.folder)), ["priority", "turn"])
        self.assertEqual(self.lines[0], "  compile lock: taken, no wait")
        # A lock file missing there can't be made: fail open, once.
        os.chmod(self.folder, 0o755)
        os.unlink(os.path.join(self.folder, "priority"))
        os.chmod(self.folder, 0o555)
        if os.geteuid() != 0:  # root creates files anyway
            self.lines.clear()
            with self.turn("pr") as turn:
                self.assertFalse(turn.held)
            self.assertEqual(len(self.lines), 1)
            self.assertTrue(self.lines[0].endswith("compiling without it"))

    @needs_flock
    def test_a_priority_job_that_could_not_announce_itself_says_so(self):
        gate = self.turn("pr")
        gate.open()
        self.assertTrue(gate.lock(gate.priority, gate.fcntl.LOCK_EX))  # a pull request between its steps, stuck
        main = self.turn("main", job="parity-main")
        self.assertTrue(main.acquire())
        self.assertIn("could not announce this job ahead of pull requests for 20 min", self.lines[0])
        self.assertTrue(self.lines[-1].startswith("  compile lock: taken after waiting 20.0 min, without the "
                                                  "announcement ahead of pull requests"), self.lines)
        main.release()

    @needs_flock
    def test_a_priority_jobs_waits_share_one_budget(self):
        # parity-main holds the turn throughout. A Mac build's first wait takes 40 min, its second gets the 20 left,
        # its third none: one try, then compiling without the turn. An unreadable record counts as 0 waited.
        self.holder_process("main", dict(MAC, GITHUB_WORKFLOW="parity-main", GITHUB_JOB="parity-main"))
        temp = Path(self.temp.name)
        environ = dict(MAC, RUNNER_TEMP=str(temp))
        budget = temp / compile_lock.BUDGET_FILE
        budget.write_text("2400")
        self.assertFalse(self.turn("build", environ=environ).acquire())
        self.assertEqual(self.clock.slept, 20 * 60)
        self.assertIn("waited 20 min, the rest of this job's 60 min; compiling without the turn", self.lines[-1])
        self.assertEqual(int(budget.read_text()), 3600)
        self.clock.slept = 0
        self.lines.clear()
        self.assertFalse(self.turn("build", environ=environ).acquire())
        self.assertEqual(self.clock.slept, 0)
        self.assertEqual(len(self.lines), 2, self.lines)
        self.assertIn("this job's 60 min of waiting are used up", self.lines[0])
        self.assertTrue(self.lines[1].startswith("  compile lock: compiling without the turn while parity-main"))
        # Digits only, at most one trailing newline, within BUDGET_MAX bytes; an overlong record with a numeric
        # prefix counts as 0 too.
        for text in ("x", "-5", "\x00", "123456789", "12 ", "12\n\n", "١٢", "60" + "0" * 30,
                     "600" + "x" * 40):
            with self.subTest(text):
                budget.write_text(text, encoding="utf-8")
                self.assertEqual(self.turn("build", environ=environ).spent(), 0)
        budget.write_bytes(b"125\n")
        self.assertEqual(self.turn("build", environ=environ).spent(), 125)
        # A pull request's job keeps its own limit per wait and leaves the record alone.
        budget.write_text("3600")
        self.clock.slept = 0
        self.assertFalse(self.turn("pr", environ=environ).acquire())
        self.assertEqual(self.clock.slept, WAIT["pr"])
        self.assertEqual(budget.read_text(), "3600")

    def reset_folder(self):
        if os.path.islink(self.folder) or os.path.isfile(self.folder):
            os.unlink(self.folder)
        elif os.path.isdir(self.folder):
            for name in os.listdir(self.folder):
                path = os.path.join(self.folder, name)
                os.rmdir(path) if os.path.isdir(path) and not os.path.islink(path) else os.unlink(path)
            os.rmdir(self.folder)

    @needs_flock
    def test_the_holders_text_reaches_a_log_only_through_the_allow_list(self):
        process = self.holder_process("main")
        text = b"##[error]boom ::error::x \x1b[31mred\x1b[0m\n::warning::y \xff\xfe" + b"z" * 5000
        Path(self.folder, "holder").write_bytes(text)
        self.turn("build").acquire()
        process.kill()
        for line in self.lines:
            self.assertTrue(line.startswith("  compile lock: "))  # so `::` can't start a line
            for marker in ("##[", "\x1b", "\n", "\r"):
                self.assertNotIn(marker, line)
            self.assertLess(len(line), 600)
        self.assertIn("##?error?boom ::error::x ??31mred??0m?::warning::y ??zzz", self.lines[0])

    def test_clean_keeps_only_the_allow_list(self):
        self.assertEqual(compile_lock.clean("a-Z_0.9:/#@ ok"), "a-Z_0.9:/#@ ok")
        self.assertEqual(compile_lock.clean("##[error]x\n::y\x1b[0m"), "##?error?x?::y??0m")

    def test_a_no_op_off_the_mac(self):
        cases = [(MAC, "linux"), (MAC, "win32"), (dict(MAC, RUNNER_ENVIRONMENT="github-hosted"), "darwin"),
                 ({}, "darwin")]
        for environ, platform in cases:
            turn = Turn("main", environ=environ, platform=platform, say=self.lines.append)
            with turn:
                self.assertFalse(turn.held)
        with self.turn("off") as turn:
            self.assertFalse(turn.held)
        self.assertFalse(os.path.exists(self.folder))
        self.assertEqual(self.lines, [])

    def test_imports_and_fails_open_without_fcntl(self):
        # Windows has no fcntl: importing the module (and tools/parity.py) works, a turn fails open.
        script = textwrap.dedent("""
            import sys
            sys.modules["fcntl"] = None
            sys.path.insert(0, sys.argv[1])
            from tools import parity
            from tools.compile_lock import Turn
            lines = []
            with Turn("main", folder=sys.argv[2], say=lines.append) as turn:
                assert not turn.held
            assert len(lines) == 1 and lines[0].endswith("compiling without it"), lines
        """)
        result = subprocess.run([sys.executable, "-c", script, str(ROOT), self.folder], capture_output=True,
                                text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stderr)

    # The wrapper

    def test_the_wrapper_returns_its_commands_status(self):
        with mock.patch.object(compile_lock.signal, "signal"):
            self.assertEqual(compile_lock.main(["off", "--", sys.executable, "-c", "raise SystemExit(3)"]), 3)
            self.assertEqual(compile_lock.main(["build"]), 2)
            self.assertEqual(compile_lock.main(["--probe"]), 0)  # what tools/ci/compile-lock.sh checks first

    def test_the_wrapper_says_once_that_it_takes_no_turn_off_the_mac(self):
        environ = {key: value for key, value in os.environ.items()
                   if key not in ("HORIZON_COMPILE_LOCK_DIR", "RUNNER_ENVIRONMENT")}
        result = subprocess.run([sys.executable, str(ROOT / "tools/compile_lock.py"), "pr", "--", sys.executable,
                                 "-c", "raise SystemExit(5)"], env=environ, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 5)
        self.assertEqual(result.stdout, "  compile lock: not the self-hosted Mac: no turn taken\n")

    def cancelled_wrapper(self, signals):
        """A wrapper whose command's child ignores SIGTERM and has closed stderr; `signals` are sent to the wrapper
        a moment apart, as a cancelled job sends them. Returns the wrapper's status, the seconds it took, and the
        child's pid."""
        pid_file = Path(self.temp.name, "child.pid")
        child = ("import os, signal, sys, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); os.close(2); "
                 f"open({str(pid_file)!r}, 'w').write(str(os.getpid())); time.sleep(120)")
        command = f"import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', {child!r}]); time.sleep(120)"
        script = textwrap.dedent("""
            import sys
            sys.path.insert(0, sys.argv[1])
            from tools import compile_lock
            compile_lock.GRACE = 5
            sys.exit(compile_lock.main(sys.argv[2:]))
        """)
        wrapper = subprocess.Popen([sys.executable, "-c", script, str(ROOT), "build", "--", sys.executable, "-c",
                                    command], env=dict(os.environ, HORIZON_COMPILE_LOCK_DIR=self.folder),
                                   stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (wrapper.poll() is None and wrapper.kill(), wrapper.stdout.close()))
        self.assertIn("taken", wrapper.stdout.readline())
        for _ in range(100):
            if pid_file.exists() and pid_file.read_text():
                break
            time.sleep(0.1)
        pid = int(pid_file.read_text())
        started = time.monotonic()
        for number in signals:
            wrapper.send_signal(number)
            time.sleep(0.5)
        status = wrapper.wait(timeout=60)
        return status, time.monotonic() - started, pid

    def assert_gone(self, pid):
        if not process_gone(pid):
            os.kill(pid, signal.SIGKILL)
            self.fail(f"process {pid} outlived the wrapper")

    @needs_flock
    def test_a_cancelled_wrapper_ends_its_commands_group_and_frees_the_turn(self):
        # SIGTERM alone: the child survives SIGTERM, so the group ends at SIGKILL after the 5 s grace.
        status, seconds, pid = self.cancelled_wrapper([signal.SIGTERM])
        self.assertEqual(status, 128 + signal.SIGTERM)
        self.assertGreaterEqual(seconds, 4)
        self.assertLess(seconds, 20)
        self.assert_gone(pid)
        self.assertTrue(self.turn("pr").acquire())

    @needs_flock
    def test_a_second_signal_cuts_the_grace_short_and_never_the_cleanup(self):
        # SIGINT, then SIGTERM during the grace: SIGKILL at once, the group gone before the wrapper exits.
        status, seconds, pid = self.cancelled_wrapper([signal.SIGINT, signal.SIGTERM])
        self.assertEqual(status, 128 + signal.SIGINT)
        self.assertLess(seconds, 4)
        self.assert_gone(pid)
        self.assertTrue(self.turn("pr").acquire())

    @unittest.skipUnless(os.name == "posix", "process groups and signals")
    def test_a_signal_during_cleanup_hurries_a_group_stop_in_another_thread(self):
        # parity.py's watchdog stops the group on its own thread while the main thread's cleanup waits for it: a
        # signal then must end that stop's grace (the normal 10 s) at once. In a process of its own, as the
        # escalation is never cleared.
        script = textwrap.dedent("""
            import os, signal, subprocess, sys, threading, time
            sys.path.insert(0, sys.argv[1])
            from tools import compile_lock
            assert compile_lock.GRACE == 10
            child = ("import os, signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                     "print(os.getpid(), flush=True); time.sleep(60)")
            group = subprocess.Popen([sys.executable, "-c", child], stdout=subprocess.PIPE, text=True,
                                     start_new_session=True)
            print(group.stdout.readline().strip(), flush=True)
            stopper = threading.Thread(target=compile_lock.stop_group, args=(group,))
            with compile_lock.Hurry():
                stopper.start()
                time.sleep(1)
                signalled = time.monotonic()
                os.kill(os.getpid(), signal.SIGTERM)
                stopper.join()
            print(f"{time.monotonic() - signalled:.2f}", flush=True)
            group.wait()
        """)
        runner = subprocess.run([sys.executable, "-c", script, str(ROOT)], capture_output=True, text=True,
                                timeout=60)
        self.assertEqual(runner.returncode, 0, runner.stderr)
        pid, seconds = runner.stdout.split()
        self.assertLess(float(seconds), 3)  # without the signal reaching the thread: about 9
        self.assertTrue(process_gone(int(pid)))

    @unittest.skipUnless(os.name == "posix", "process groups and signals")
    def test_a_hurried_group_stop_ends_soon_even_when_ps_never_answers(self):
        # Every ps stalls, after SIGKILL too, and the group ignores SIGTERM: a signal during the grace (normal 10 s,
        # probes normally 5 s each) ends the probe under way, SIGKILL follows, and the stop returns within
        # HURRIED_KILL_WAIT of it, though kill(-pgid, 0) keeps answering for the unreaped leader.
        script = textwrap.dedent("""
            import os, signal, subprocess, sys, threading, time
            sys.path.insert(0, sys.argv[1])
            from tools import compile_lock
            assert (compile_lock.GRACE, compile_lock.PROBE_TIMEOUT, compile_lock.HURRIED_KILL_WAIT) == (10, 5, 2)
            child = ("import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print(1, flush=True); "
                     "time.sleep(60)")
            group = subprocess.Popen([sys.executable, "-c", child], stdout=subprocess.PIPE, text=True,
                                     start_new_session=True)
            group.stdout.readline()
            popen, probes = subprocess.Popen, []
            def stalled(command, *args, **options):
                if command[0] == "ps":
                    probes.append(time.monotonic())
                    command = [sys.executable, "-c", "import time; time.sleep(60)"]
                return popen(command, *args, **options)
            compile_lock.subprocess.Popen = stalled
            signalled = []
            def second_signal():
                signalled.append(time.monotonic())
                os.kill(os.getpid(), signal.SIGTERM)
            threading.Timer(1, second_signal).start()
            compile_lock.stop_group(group)
            print(f"{time.monotonic() - signalled[0]:.2f} {len(probes)} {group.wait()}", flush=True)
        """)
        runner = subprocess.run([sys.executable, "-c", script, str(ROOT)], capture_output=True, text=True,
                                timeout=60)
        self.assertEqual(runner.returncode, 0, runner.stderr)
        seconds, probes, status = runner.stdout.split()
        self.assertLess(float(seconds), 3)
        self.assertGreater(int(probes), 1, "no probe ran after SIGKILL")
        self.assertEqual(int(status), -signal.SIGKILL)

    def test_the_module_runs_on_the_macs_system_python(self):
        ast.parse(Path(compile_lock.__file__).read_text(encoding="utf-8"), feature_version=(3, 9))

    def test_the_mac_compiles_take_their_turn(self):
        with mock.patch("sys.path", [str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"), *sys.path]):
            import yaml

        def load(name):
            return yaml.safe_load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"))
        game = load("game-build.yml")
        self.assertEqual(game["env"]["HORIZON_COMPILE_LOCK_ROLE"], "build")  # vanilla-data.sh's preparer build too
        jobs = game["jobs"]
        # Each Mac cargo compile is its step's entry process (exec), in the turn: the preparer, the game, xtask.
        wrapped = "exec bash tools/ci/compile-lock.sh build -- cargo build --release --locked -p horizon-prepare"
        steps = [step.get("run", "") for step in jobs["exporters"]["steps"]]
        self.assertTrue(any(run.rstrip().endswith(wrapped) for run in steps))
        compiles = [run for run in steps if "cargo build" in run]
        self.assertEqual(len(compiles), 1, "no other cargo build in exporters")
        build = {step.get("name"): step for step in jobs["build"]["steps"]}
        game_build = build["Build the official game, horizon-ctl, horizon-prepare and horizon-server"]["run"]
        self.assertIn('if [ -f tools/ci/compile-lock.sh ]; then lock="bash tools/ci/compile-lock.sh build --"',
                      game_build)
        self.assertIn('if [ "${{ matrix.mac }}" = true ]; then', game_build)
        self.assertIn("exec $lock cargo build --release --locked", game_build)
        xtask = build["The packaging tool, compiled in its turn"]
        self.assertEqual(xtask["if"], "matrix.mac")
        self.assertTrue(xtask["run"].rstrip().endswith("exec bash tools/ci/compile-lock.sh build -- cargo build "
                                                       "--locked -p xtask"))
        names = list(build)
        self.assertLess(names.index("The packaging tool, compiled in its turn"), names.index("Package"))
        # Room for the job's 60 min of waiting: 150 min for exporters, 180 for the Mac build legs.
        self.assertEqual(jobs["exporters"]["timeout-minutes"], 150)
        self.assertTrue(jobs["build"]["timeout-minutes"].startswith("${{ matrix.mac && 180 ||"))
        main = load("parity-main.yml")["jobs"]["parity-main"]
        # 20 min of waiting and the 240 min prebuild limit (tools/parity.py) leave setup and the harnesses 100.
        self.assertEqual(main["timeout-minutes"], 360)
        self.assertEqual(main["env"]["HORIZON_COMPILE_LOCK_ROLE"], "main")
        harnesses = next(step for step in main["steps"] if step.get("id") == "harnesses")
        self.assertIn("\nexec python3 tools/parity.py --changed", harnesses["run"])
        verdict = next(step for step in main["steps"] if "verdict" in step.get("run", ""))
        self.assertEqual(verdict["env"]["HORIZON_COMPILE_LOCK_ROLE"], "off")
        parity = [step.get("run", "") for step in load("parity.yml")["jobs"]["parity"]["steps"]]
        self.assertEqual(len([run for run in parity if run.startswith("exec python3 tools/parity.py")]), 2)
        # The launcher's Mac leg on main: tests and launcher in one turn, before the steps that run them; branch
        # builds (GitHub's Macs) never run it.
        launcher = load("launcher.yml")["jobs"]["build"]
        names = [step.get("name") for step in launcher["steps"]]
        turn = launcher["steps"][names.index("The launcher and its tests, compiled in their turn")]
        self.assertEqual(turn["if"], "matrix.mac && needs.decide.outputs.main == 'true'")
        self.assertIn("exec bash ../tools/ci/compile-lock.sh build -- bash -c \"$compile\"", turn["run"])
        self.assertIn("cargo test --release --locked --no-run", turn["run"])
        self.assertLess(names.index("The launcher and its tests, compiled in their turn"),
                        names.index("Test every launcher target in a scratch home"))
        # 90 min on the Mac mini (main); GitHub's Macs (branch builds) keep 60.
        self.assertEqual(launcher["timeout-minutes"],
                         "${{ matrix.mac && needs.decide.outputs.main == 'true' && 90 || matrix.mac && 60 || 40 }}")
        vanilla = (ROOT / "tools/ci/vanilla-data.sh").read_text(encoding="utf-8")
        self.assertIn('in_turn="bash tools/ci/compile-lock.sh ${HORIZON_COMPILE_LOCK_ROLE:-pr} --"', vanilla)
        self.assertEqual(vanilla.count("$in_turn "), 2)
        # `lock` stays the preparation lock's folder, which the mkdir fallback's EXIT trap removes.
        self.assertEqual([line.strip() for line in vanilla.splitlines() if line.strip().startswith("lock=")],
                         ['lock="$shared/prepared/.lock"'])
        self.assertEqual(vanilla.count("cargo build"), 2, "no cargo build outside the turn")


if __name__ == "__main__":
    unittest.main()
