"""The headless-test wrapper supplies a real ready display and owns its lifetime."""
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "tools/ci/with_xvfb.py"
# An unauthenticated local X11 setup request: the success byte means the server
# accepted the connection, not merely that a stale socket or DISPLAY exists.
CONNECT = """
import json, os, socket, struct, sys
number = os.environ['DISPLAY'].removeprefix(':')
with socket.socket(socket.AF_UNIX) as connection:
    connection.connect('/tmp/.X11-unix/X' + number)
    connection.sendall(struct.pack('<BBHHHHH', ord('l'), 0, 11, 0, 0, 0, 0))
    assert connection.recv(1) == b'\\x01'
print(json.dumps({'display': number, 'wayland': os.getenv('WAYLAND_DISPLAY'),
                  'authority': os.getenv('XAUTHORITY')}), flush=True)
sys.exit(int(sys.argv[1]))
"""


@unittest.skipIf(os.name == "nt", "the wrapper passes a file descriptor to Xvfb, for Linux CI")
class XvfbTests(unittest.TestCase):
    # These integration checks need a real server; the fake-server failure check
    # below still runs in offline tooling environments without Xvfb installed.
    @unittest.skipUnless(shutil.which("Xvfb"), "Xvfb is required for the real-display integration test")
    def test_ready_display_replaces_inherited_session_and_is_cleaned_up(self):
        env = dict(os.environ, DISPLAY=":9999", WAYLAND_DISPLAY="owner-display",
                   XAUTHORITY="/nonexistent/owner-authority")
        result = subprocess.run([sys.executable, WRAPPER, sys.executable, "-c", CONNECT, "0"],
                                env=env, text=True, capture_output=True, timeout=45)
        self.assertEqual(result.returncode, 0, result.stderr)
        display = json.loads(result.stdout)
        self.assertNotEqual(display["display"], "9999")
        self.assertIsNone(display["wayland"])
        self.assertIsNone(display["authority"])
        self.assertFalse(Path("/tmp/.X11-unix/X" + display["display"]).exists())

    @unittest.skipUnless(shutil.which("Xvfb"), "Xvfb is required for the real-display integration test")
    def test_command_failure_is_preserved_and_display_is_cleaned_up(self):
        result = subprocess.run([sys.executable, WRAPPER, sys.executable, "-c", CONNECT, "7"],
                                text=True, capture_output=True, timeout=45)
        self.assertEqual(result.returncode, 7, result.stderr)
        display = json.loads(result.stdout)
        self.assertFalse(Path("/tmp/.X11-unix/X" + display["display"]).exists())

    def test_failed_server_never_runs_the_command(self):
        with tempfile.TemporaryDirectory() as directory:
            server = Path(directory) / "Xvfb"
            server.write_text("#!/bin/sh\nexit 3\n")
            server.chmod(0o755)
            marker = Path(directory) / "command-ran"
            result = subprocess.run(
                [sys.executable, WRAPPER, sys.executable, "-c",
                 "from pathlib import Path; import sys; Path(sys.argv[1]).touch()", str(marker)],
                env=dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"]),
                text=True, capture_output=True, timeout=5,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("without reporting a ready display", result.stderr)
            self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
