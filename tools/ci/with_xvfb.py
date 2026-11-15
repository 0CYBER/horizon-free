#!/usr/bin/env python3
"""Run a command against its own ready X11 display, including on a headless runner."""
import os
import select
import subprocess
import sys


def run(command):
    read_fd, write_fd = os.pipe()
    with os.fdopen(read_fd, "rb", buffering=0) as ready, os.fdopen(write_fd, "wb") as notify:
        with subprocess.Popen(
            ["Xvfb", "-displayfd", str(notify.fileno()), "-screen", "0", "1280x1024x24",
             "-nolisten", "tcp", "-noreset"],
            pass_fds=(notify.fileno(),),
        ) as server:
            notify.close()
            try:
                if not select.select([ready], [], [], 30)[0]:
                    raise SystemExit("Xvfb did not report a ready display within 30 seconds")
                display = ready.readline().strip()
                if not display.isdigit():
                    raise SystemExit("Xvfb exited without reporting a ready display")
                env = os.environ.copy()
                env["DISPLAY"] = ":" + display.decode("ascii")
                env.pop("WAYLAND_DISPLAY", None)
                env.pop("XAUTHORITY", None)
                return subprocess.run(command, env=env).returncode
            finally:
                server.terminate()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit("usage: with_xvfb.py COMMAND [ARG ...]")
    sys.exit(run(sys.argv[1:]))
