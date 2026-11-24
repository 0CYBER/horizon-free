#!/usr/bin/env python3
"""Turns at compiling for the jobs on the self-hosted Mac (#19151).

parity-main, pull requests' parity jobs and the Nightly's and releases' Mac legs ran on one machine and compiled
side by side. Each takes a turn around its heavy compile.

Lock files in LOCK_DIR, held with flock, so the kernel ends a turn the moment its holder dies:
  - `turn`: held exclusively by the job compiling.
  - `priority`: parity-main and the Mac builds (PRIORITY roles) hold it shared from their arrival until their compile
    ends. A pull request's job (role `pr`, also the nightly parity audit) takes `turn` only while it holds
    `priority` exclusively, then lets `priority` go again.
  - `waiting` (optional): a priority job holds it shared only while it waits for `turn`, from its first refused try
    until it takes `turn` or stops waiting. Where it can't be used, said once, everything else works as before and
    no pull request gives the turn back.

Admission order of a `pr` job, all non-blocking: (1) flock(priority, EX); if refused, a priority job is here: wait.
(2) flock(turn, EX). (3) flock(priority, UN), whether (2) succeeded or not. A priority job's arrival is its shared
lock on `priority`, which it keeps. An exclusive and a shared lock on one file never coexist, so in the instant a
`pr` job takes `turn` no priority job has arrived. One that arrives later waits for `turn`, and a `pr` job holding it
gives it back at its next cargo-call boundary (Turn.yield_if_ahead, which tools/parity.py's prebuild calls between
its Cargo runs; a wrapped command gives nothing back) when `waiting` is refused to it exclusively: a priority job
waits for the turn. A granted try is let go at once, and no `pr` job locks `waiting` otherwise. A priority job that
stopped waiting and compiles without the turn holds `priority` but not `waiting`, so it takes nobody's turn away. A
waiting priority job takes `turn` before it lets `waiting` go; a `pr` job that gave the turn back can't take it
meanwhile anyway, as `priority` (held shared by that priority job until its compile ends) refuses its step (1). It
waits as any `pr` job does, admitted again only once no priority job is here. Priority jobs never give the turn back.
A pull request's job is admitted when it takes the turn; nothing here says when its cargo starts.

Priority jobs among themselves, and pull requests among themselves, poll `turn` every POLL seconds with a bounded
wait: whoever polls first after a release gets the turn, which is no queue. Every wait is bounded (WAIT, by
time.monotonic): then the job compiles anyway and says who held the turn. A priority job that goes ahead keeps its `priority` lock, so no pull
request is admitted beside it meanwhile. A priority job's waits share one budget per job (WAIT[role] in all): each
records what it waited in BUDGET_FILE under RUNNER_TEMP, and a later wait in the same job gets only what is left
(its preparer, game and packaging compiles, say). A pull request's job has WAIT["pr"] per Turn (a prebuild, a
wrapped command), in memory: its first wait and those after giving the turn back share it. A holder that hangs is
only ever met by these bounds: nothing is taken over, renamed or deleted here, ever.

Fail open: a folder or lock file that can't be opened, isn't a real directory or regular file, or a system without
fcntl (Windows): said once, and the job compiles without the lock. The folder and the lock files may already exist
and be read-only to the job: they are opened as they are, and made only where missing and allowed.

The `holder` file is display only: who holds the turn and since when, for the waiters' log lines. Nothing is
decided from it. Its reader caps it at INFO_MAX bytes and passes it through an allow-list of characters.

If a lock file is replaced while held, two compiles can overlap, as before this change.

Active only where RUNNER_ENVIRONMENT is self-hosted on macOS (the runner sets it for every job), or where
HORIZON_COMPILE_LOCK_DIR names a folder (tests). The role `off` leaves a job out.

Usage as a wrapper (game-build.yml and tools/ci/vanilla-data.sh, through tools/ci/compile-lock.sh): this process
holds the turn and runs the command as a child in its own process group, which never inherits the lock's
descriptors. Signalled (a cancelled job: SIGINT, then SIGTERM), it ends that whole group before it lets the turn go:
  python3 tools/compile_lock.py build -- cargo build --release ...
"""

import os
import re
import signal
import stat
import subprocess
import sys
import threading
import time

LOCK_DIR = "/Users/Shared/horizon-compile-lock"
# Each role's longest wait, in seconds: for a priority role, all of a job's waits together (BUDGET_FILE); for a pull
# request, all waits of one Turn (a prebuild's first and those after giving the turn back). parity-main 20 min of its
# job's 360; a Mac build job 60 (Mac build legs 180, exporters 150, the launcher's Mac leg on main 90); a pull
# request's parity job 60 per prebuild of its 240 (the frame_identity step is a second prebuild).
WAIT = {"main": 20 * 60, "build": 60 * 60, "pr": 60 * 60}
PRIORITY = {"main", "build"}
BUDGET_FILE = "horizon-compile-lock-waited"  # under RUNNER_TEMP: the seconds this job's priority waits took
BUDGET_MAX = 16  # bytes a budget record may have
PROBE_TIMEOUT = 5.0  # seconds a ps listing of a process group may take
HURRIED_KILL_WAIT = 2.0  # seconds a hurried group stop waits after SIGKILL, its probes included
POLL = 2.0
ARRIVE_POLL = 0.1  # a pull request holds `priority` exclusively only for an instant
INFO_MAX = 300
NOT_ALLOWED = re.compile(r"[^A-Za-z0-9 \-_.:/#@]")
PREFIX = "  compile lock: "
GRACE = 10.0  # between SIGTERM and SIGKILL for a stopped command's process group
KILL_WAIT = 5.0  # for the group to be gone after SIGKILL


class Odd(Exception):
    """A lock folder entry that is not what this module makes."""


def clean(text, limit=120):
    """Only letters, digits, space and - _ . : / # @; the rest becomes ?. Every line also starts with PREFIX."""
    return NOT_ALLOWED.sub("?", str(text))[:limit]


def active(environ, platform):
    return platform == "darwin" and environ.get("RUNNER_ENVIRONMENT") == "self-hosted"


def open_folder(path):
    """A descriptor on the lock folder, made (mode 1777, like /tmp) only if missing; Odd unless a real directory."""
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    created = False
    try:
        descriptor = os.open(path, flags)
    except FileNotFoundError:
        try:
            os.mkdir(path, 0o1777)
            created = True
        except FileExistsError:
            pass
        descriptor = os.open(path, flags)
    try:
        if not stat.S_ISDIR(os.fstat(descriptor).st_mode):
            raise Odd("the lock folder is not a directory")
        if created:
            try:
                os.fchmod(descriptor, 0o1777)  # the umask narrowed mkdir's mode
            except OSError:
                pass
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def open_entry(folder, name, write=False):
    """A descriptor on folder/name, a regular file with one link, opened as it is, or made (0666) only where
    missing and the folder allows it. Never follows a link and never blocks (a FIFO opens at once and is refused).
    Odd for anything else. Every check is on the open descriptor, never on the path."""
    if folder is None:  # dir_fd=None would mean the current directory
        raise Odd(f"no lock folder is open for `{name}`")
    access = (os.O_WRONLY if write else os.O_RDONLY) | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    try:
        descriptor = os.open(name, access, dir_fd=folder)
    except FileNotFoundError:
        try:
            descriptor = os.open(name, access | os.O_CREAT | os.O_EXCL, 0o666, dir_fd=folder)
            try:
                os.fchmod(descriptor, 0o666)  # only a file this job made; the umask narrowed its mode
            except OSError:
                pass
        except FileExistsError:  # another job made it meanwhile
            descriptor = os.open(name, access, dir_fd=folder)
    try:
        status = os.fstat(descriptor)
        if not stat.S_ISREG(status.st_mode) or status.st_nlink != 1:
            raise Odd(f"`{name}` is not a regular file")
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


class Turn:
    """The lock as a context manager: entering waits for a turn (bounded), leaving releases it. A pull request's
    job may give it back and wait again between its compiles (yield_if_ahead).

    clock and sleep are time.monotonic and time.sleep unless a test passes its own."""

    def __init__(self, role, folder=None, environ=None, platform=None, clock=time.monotonic, sleep=time.sleep,
                 say=None):
        environ = os.environ if environ is None else environ
        self.role = role
        temp = environ.get("RUNNER_TEMP")
        self.budget_file = os.path.join(temp, BUDGET_FILE) if temp and role in PRIORITY else None
        folder = folder or environ.get("HORIZON_COMPILE_LOCK_DIR")
        self.on = role != "off" and bool(folder or active(environ, platform or sys.platform))
        self.path = folder or LOCK_DIR
        self.clock, self.sleep = clock, sleep
        self.say_line = say or (lambda line: print(line, flush=True))
        # In the allow-list's characters only, as every reader shows it.
        self.me = clean(f"{environ.get('GITHUB_WORKFLOW') or '?'} / {environ.get('GITHUB_JOB') or '?'} - role {role} "
                        f"run {environ.get('GITHUB_RUN_ID') or '?'} runner {environ.get('RUNNER_NAME') or '?'} pid "
                        f"{os.getpid()}", 200)
        self.fcntl = self.folder = self.turn = self.priority = self.waiting = None
        self.held = self.arrived = self.ahead = self.waiting_held = False
        self.complained = False
        self.taken_at = None
        self.waited = 0  # seconds this Turn's waits took (a pull request's budget)

    def say(self, line):
        try:
            self.say_line(PREFIX + line)
        except Exception:
            pass

    def fail_open(self, error):
        if not self.complained:
            self.complained = True
            self.say(f"{clean(error, 200)}; compiling without it")

    def lock(self, descriptor, mode):
        """flock without blocking: True when taken, False while another holds it."""
        try:
            self.fcntl.flock(descriptor, mode | self.fcntl.LOCK_NB)
            return True
        except BlockingIOError:
            return False

    def holder(self):
        """Who holds the turn, from the display file: cleaned text, or "" when it says nothing usable."""
        try:
            descriptor = open_entry(self.folder, "holder")
            try:
                text = os.read(descriptor, INFO_MAX).decode("utf-8", errors="replace").strip()
            finally:
                os.close(descriptor)
            return clean(text, 200)
        except Exception:
            return ""

    def write_holder(self):
        """Display only: written through the descriptor whose checks passed (ftruncate, never by path); any
        failure leaves the display as it is."""
        try:
            descriptor = open_entry(self.folder, "holder", write=True)
            try:
                os.ftruncate(descriptor, 0)
                os.write(descriptor, f"{self.me} since {time.strftime('%H:%M:%SZ', time.gmtime())}".encode())
            finally:
                os.close(descriptor)
        except Exception:
            pass  # display only

    def clear_holder(self):
        try:
            descriptor = open_entry(self.folder, "holder", write=True)
            try:
                os.ftruncate(descriptor, 0)
            finally:
                os.close(descriptor)
        except Exception:
            pass

    # The turn
    def open(self):
        import fcntl  # here: Windows has none, and importing tools/parity.py there must keep working
        self.fcntl = fcntl
        self.folder = open_folder(self.path)
        self.turn = open_entry(self.folder, "turn")
        self.priority = open_entry(self.folder, "priority")
        try:
            self.waiting = open_entry(self.folder, "waiting")
        except Exception as error:
            self.say(f"the waiting lock file can't be used ({clean(f'{type(error).__name__}: {error}', 160)}): pull "
                     "requests keep the turn while jobs ahead of them wait")

    def arrive(self):
        """A priority job's arrival: True once it holds `priority` shared."""
        self.arrived = self.lock(self.priority, self.fcntl.LOCK_SH)
        return self.arrived

    def try_turn(self):
        """One attempt at `turn`. A pull request's job only while no priority job is here (see the module's text)."""
        self.ahead = False
        if self.role in PRIORITY:
            self.held = self.lock(self.turn, self.fcntl.LOCK_EX)
            self.taken_at = self.clock()
            return self.held
        if not self.lock(self.priority, self.fcntl.LOCK_EX):
            self.ahead = True
            return False
        try:
            self.held = self.lock(self.turn, self.fcntl.LOCK_EX)
            self.taken_at = self.clock()
        finally:
            self.fcntl.flock(self.priority, self.fcntl.LOCK_UN)
        return self.held

    def acquire(self):
        """Wait for a turn, at most WAIT[role]; True when this job holds it."""
        if not self.on:
            return False
        if self.role not in WAIT:
            self.fail_open(f"unknown role {self.role}")
            return False
        spent = self.spent()
        started = self.clock()
        try:
            self.open()
            return self.wait(max(0, WAIT[self.role] - spent), spent)
        except Exception as error:
            self.fail_open(f"{self.path} can't be used ({type(error).__name__}: {error})")
            self.close()
            return False
        finally:
            self.stop_waiting()  # after `turn` was taken, if it was (see the module's text)
            self.waited = self.clock() - started
            self.record(spent + self.waited)

    def stop_waiting(self):
        """A priority job lets `waiting` go: it took the turn, or stopped waiting for it."""
        if self.waiting_held and self.waiting is not None:
            try:
                self.fcntl.flock(self.waiting, self.fcntl.LOCK_UN)
            except OSError:
                pass  # closing the descriptor unlocks it
        self.waiting_held = False

    def yield_if_ahead(self):
        """At a cargo-call boundary: a pull request's job holding the turn gives it back while a priority job waits
        (see the module's text), then waits for it again with what is left of WAIT["pr"]. True when it gave the turn
        back. Without the turn held (the lock off or failed open included), without `waiting`, or for a priority role,
        it does nothing."""
        if not self.held or self.role in PRIORITY or self.waiting is None:
            return False
        try:
            if self.lock(self.waiting, self.fcntl.LOCK_EX):
                self.fcntl.flock(self.waiting, self.fcntl.LOCK_UN)
                return False
            held_for = self.clock() - self.taken_at
            self.clear_holder()
            self.fcntl.flock(self.turn, self.fcntl.LOCK_UN)
            self.held = False
            self.say(f"gave the turn back after {held_for / 60:.1f} min: a job ahead of pull requests is waiting")
            started = self.clock()
            try:
                self.wait(max(0, WAIT[self.role] - self.waited), int(self.waited))
            finally:
                self.waited += self.clock() - started
        except Exception as error:
            self.fail_open(f"{self.path} can't be used ({type(error).__name__}: {error})")
            if self.held:
                self.clear_holder()
            self.held = False
            self.close()
        return True

    def spent(self):
        """Seconds this job's earlier priority waits took: 0 without a budget file or for one that isn't a number."""
        if not self.budget_file:
            return 0
        try:
            with open(self.budget_file, "rb") as handle:
                data = handle.read(BUDGET_MAX + 1)
        except OSError:
            return 0
        # Digits only, one trailing newline at most; anything longer than BUDGET_MAX is not a record of ours.
        text = data[:-1] if data.endswith(b"\n") else data
        if len(data) > BUDGET_MAX or not re.fullmatch(rb"[0-9]{1,8}", text):
            return 0
        return int(text)

    def record(self, seconds):
        if not self.budget_file:
            return
        try:
            with open(self.budget_file, "w", encoding="utf-8") as handle:
                handle.write(str(int(seconds)))
        except OSError:
            pass

    def wait(self, limit, spent=0):
        started = self.clock()
        total = WAIT[self.role]
        whose = "this job's" if self.role in PRIORITY else "this prebuild's"
        if limit <= 0:
            self.say(f"{whose} {total // 60:.0f} min of waiting are used up: one try for the turn, else compiling "
                     "without it")
        if self.role in PRIORITY:
            while not self.arrive():
                if self.clock() - started >= limit:
                    if limit <= 0:
                        break
                    self.say(f"could not announce this job ahead of pull requests for {limit // 60:.0f} min (a "
                             "pull request held `priority` at every try); trying for the turn as it is")
                    break
                self.sleep(ARRIVE_POLL)
        waiting = False
        while not self.try_turn():
            holder = self.holder()
            if self.ahead:
                blocker = "jobs ahead of pull requests (parity-main, a Mac build) are here" + \
                    (f"; {holder} holds the turn" if holder else "")
            else:
                blocker = f"{holder or 'an unknown job'} holds the turn"
            if not waiting and limit > 0:
                self.say(f"waiting: {blocker}")
                waiting = True
            if self.clock() - started >= limit:
                if limit <= 0:
                    self.say(f"compiling without the turn while {blocker}")
                else:
                    share = f"{whose} limit" if not spent else f"the rest of {whose} {total // 60:.0f} min"
                    self.say(f"waited {limit / 60:.0f} min, {share}; compiling without the turn while {blocker}")
                return False
            if self.role in PRIORITY and not self.waiting_held and self.waiting is not None:
                try:
                    self.waiting_held = self.lock(self.waiting, self.fcntl.LOCK_SH)  # refused only for an instant
                except OSError:
                    pass  # optional, as at open: this job waits as on main, unseen by pull requests
            self.sleep(POLL)
        self.write_holder()
        waited = self.taken_at - started
        unannounced = "" if self.arrived or self.role not in PRIORITY else ", without the announcement ahead of " \
            "pull requests"
        self.say((f"taken after waiting {waited / 60:.1f} min" if waiting or waited >= 1 else "taken, no wait")
                 + unannounced)
        return True

    def close(self):
        self.waiting_held = False
        for name in ("turn", "priority", "waiting", "folder"):
            descriptor = getattr(self, name)
            setattr(self, name, None)
            if descriptor is not None:
                try:
                    os.close(descriptor)  # closing also unlocks
                except OSError:
                    pass

    def release(self):
        if not self.on:
            return
        if self.held and self.folder is not None:
            self.clear_holder()
            self.say(f"released after {(self.clock() - self.taken_at) / 60:.1f} min")
        self.held = self.arrived = False
        self.close()

    def __enter__(self):
        self.acquire()
        return self

    def __exit__(self, *exception):
        self.release()
        return False


# Set by a signal that arrives during cleanup (Hurry): every group stop under way, in any thread (parity.py's
# watchdog), skips the rest of its grace and sends SIGKILL. Never cleared: a process that got it is ending.
ESCALATE = threading.Event()


class Hurry:
    """While cleanup runs in the main thread, SIGINT, SIGTERM and SIGHUP only set ESCALATE, which cuts every group
    stop's grace short: a second signal from a cancelled job never interrupts the cleanup itself. Entered in another
    thread, it installs nothing."""

    NAMES = ("SIGINT", "SIGTERM", "SIGHUP")

    def __init__(self):
        self.event = ESCALATE
        self.saved = {}

    def __enter__(self):
        if threading.current_thread() is threading.main_thread():
            for name in self.NAMES:
                if hasattr(signal, name):
                    number = getattr(signal, name)
                    try:
                        self.saved[number] = signal.signal(number, lambda *_: self.event.set())
                    except (ValueError, OSError):
                        pass
        return self.event

    def __exit__(self, *exception):
        for number, handler in self.saved.items():
            try:
                signal.signal(number, handler)
            except (ValueError, OSError):
                pass
        return False


def group_alive(pgid, timeout=PROBE_TIMEOUT, hurry=None):
    """Whether anything but a zombie is left in the process group. ps (macOS and Linux) lists the group's processes
    with their state; it is given up after `timeout` seconds, or as soon as `hurry` is set. Without its answer,
    kill(-pgid, 0), which a zombie leader alone still answers."""
    try:
        probe = subprocess.Popen(["ps", "-A", "-o", "pgid=,stat="], stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL, text=True)
    except OSError:
        probe = None
    if probe:
        with probe:
            end, listing = time.monotonic() + timeout, None
            while listing is None:
                try:
                    listing, _ = probe.communicate(timeout=max(0, min(0.1, end - time.monotonic())))
                except subprocess.TimeoutExpired:
                    if time.monotonic() >= end or (hurry is not None and hurry.is_set()):
                        probe.kill()
                        break
            if listing is not None and probe.returncode == 0:
                return any(fields[0] == str(pgid) and not fields[1].startswith("Z")
                           for fields in (line.split() for line in listing.splitlines()) if len(fields) >= 2)
    try:
        os.killpg(pgid, 0)
        return True
    except (ProcessLookupError, PermissionError):
        return False


def wait_exited(process):
    """Block until the process has exited, without reaping it (os.waitid WNOWAIT), so its process group's id can't
    name another group yet. Without waitid (macOS before Python 3.13) it returns at once."""
    if os.name == "nt" or not hasattr(os, "waitid") or process.returncode is not None:
        return
    try:
        os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT)
    except ChildProcessError:
        pass


def stop_group(process, grace=None):
    """End the process and everything in its process group, and return only once the group is gone (or SIGKILL
    has been sent and KILL_WAIT has passed): SIGTERM, up to `grace` seconds (GRACE; a further SIGINT, SIGTERM or
    SIGHUP cuts it short), then SIGKILL. The leader is not reaped here, so the group's id can't name another group
    while this runs; the caller reaps it after."""
    if os.name == "nt":
        process.kill()
        return
    if process.returncode is not None:
        return
    with Hurry() as hurry:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            return
        deadline = time.monotonic() + (GRACE if grace is None else grace)
        while time.monotonic() < deadline and not hurry.is_set():
            if not group_alive(process.pid, hurry=hurry):
                return
            hurry.wait(0.2)
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            return
        # Up to KILL_WAIT for the group to go. Once hurried, HURRIED_KILL_WAIT from then, probes included: a probe
        # that doesn't answer falls back to kill(-pgid, 0), which the unreaped leader keeps answering.
        deadline, hurried = time.monotonic() + KILL_WAIT, False
        while True:
            now = time.monotonic()
            if not hurried and hurry.is_set():
                deadline, hurried = min(deadline, now + HURRIED_KILL_WAIT), True
            if now >= deadline:
                return
            if hurried:
                alive = group_alive(process.pid, timeout=deadline - now)
            else:
                alive = group_alive(process.pid, hurry=hurry)
            if not alive:
                return
            time.sleep(0.1)


def main(argv):
    """compile_lock.py <role> -- <command>: run the command in a turn; its exit status is this one's.
    compile_lock.py --probe: exit 0, so tools/ci/compile-lock.sh knows this Python can run the module."""
    if argv == ["--probe"]:
        return 0
    if len(argv) < 3 or argv[1] != "--":
        print("usage: compile_lock.py <main|pr|build|off> -- <command>...", file=sys.stderr)
        return 2
    role, command = argv[0], argv[2:]
    # A cancelled job's SIGTERM (or SIGINT, SIGHUP) ends this process through the finally blocks below, which stop
    # the command's whole process group and then release the turn.
    for name in ("SIGTERM", "SIGINT", "SIGHUP"):
        if hasattr(signal, name):
            signal.signal(getattr(signal, name), lambda received, _frame: sys.exit(128 + received))
    turn = Turn(role)
    if not turn.on and role != "off":
        print(f"{PREFIX}not the self-hosted Mac: no turn taken", flush=True)
    with turn:
        process = subprocess.Popen(command, start_new_session=os.name != "nt")
        try:
            return process.wait()
        except BaseException:
            stop_group(process)  # its whole group first, then the leader is reaped, then the turn released
            process.wait()
            raise


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
