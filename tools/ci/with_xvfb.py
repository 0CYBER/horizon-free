#!/usr/bin/env python3
"""Run a command against its own ready X11 display, including on a headless runner."""
import os
import select
import subprocess
import sys

# The first display tried, as tools/ci/linux-runner.sh picks its own: well above the ones a
# desktop session uses.
FIRST = 90
TRIES = 20


def taken(number):
    """Whether something already holds display `number`: its lock file, its socket (also an
    Xwayland's path socket `X<n>_`), or an abstract socket. Xvfb's own `-displayfd` search only
    asked for the lock and the plain socket, so on a desktop whose Xwayland had only `X0_` and the
    lock it took :0, replaced the session's `/tmp/.X11-unix/X0` and deleted it on exit, and every
    X11 program started afterwards found no display."""
    paths = (f"/tmp/.X{number}-lock", f"/tmp/.X11-unix/X{number}", f"/tmp/.X11-unix/X{number}_")
    if any(os.path.lexists(path) for path in paths):
        return True
    try:
        with open("/proc/net/unix", encoding="utf-8", errors="replace") as sockets:
            return any(line.rstrip().endswith(f"@/tmp/.X11-unix/X{number}") for line in sockets)
    except OSError:
        return False


def start(number):
    """Xvfb on display `number`, and the number once it reports ready; `None` where it exited
    first (another run took the display between the check and the start)."""
    read_fd, write_fd = os.pipe()
    with os.fdopen(read_fd, "rb", buffering=0) as ready, os.fdopen(write_fd, "wb") as notify:
        server = subprocess.Popen(
            ["Xvfb", f":{number}", "-displayfd", str(notify.fileno()), "-screen", "0",
             "1280x1024x24", "-nolisten", "tcp", "-noreset"],
            pass_fds=(notify.fileno(),),
        )
        notify.close()
        if not select.select([ready], [], [], 30)[0]:
            server.terminate()
            server.wait()
            raise SystemExit("Xvfb did not report a ready display within 30 seconds")
        display = ready.readline().strip()
        if not display.isdigit():
            server.wait()
            return server, None
        return server, display.decode("ascii")


def run(command):
    number, failed = FIRST, 0
    while failed < TRIES:
        if taken(number):
            number += 1
            continue
        server, display = start(number)
        if display is None:
            failed += 1
            number += 1
            continue
        try:
            env = os.environ.copy()
            env["DISPLAY"] = ":" + display
            env.pop("WAYLAND_DISPLAY", None)
            env.pop("XAUTHORITY", None)
            return subprocess.run(command, env=env).returncode
        finally:
            server.terminate()
            server.wait()
    raise SystemExit("Xvfb exited without reporting a ready display")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit("usage: with_xvfb.py COMMAND [ARG ...]")
    sys.exit(run(sys.argv[1:]))
