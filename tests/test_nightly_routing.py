"""Exercise the game smoke command without building the game."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]
SMOKE = ROOT / "tools/ci/game_smoke.sh"


class NightlyRoutingTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "The smoke command runs on Linux and macOS")
    def test_smoke_runs_from_relative_or_absolute_binary_directory(self):
        command = next(line.strip() for line in SMOKE.read_text().splitlines() if line.strip().startswith("client="))
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary_dir = root / "target tree/release"
            binary_dir.mkdir(parents=True)
            binary = binary_dir / "horizon-client"
            binary.write_text("#!/bin/sh\nprintf 'smoke ran'\n")
            binary.chmod(0o755)
            for directory in [str(binary_dir), str(binary_dir.relative_to(root))]:
                with self.subTest(directory=directory):
                    result = subprocess.run(["bash", "-c", command + '\n"$client"'], cwd=root, env={**os.environ, "BIN": directory}, text=True, capture_output=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, "smoke ran")

    @unittest.skipUnless(shutil.which("Xvfb"), "Xvfb is required for the Linux smoke display")
    def test_linux_smoke_gets_a_ready_display(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary = root / "horizon-client"
            # A controlled game fixture isolates the real smoke orchestration and Xvfb.
            binary.write_text(textwrap.dedent('''\
                #!/usr/bin/env python3
                import json, os, pathlib, sys
                args = sys.argv[1:]
                report = {"frames": int(args[args.index("--world-frames") + 1]),
                          "chunks": 1, "meshed_sections": 1, "player": [0, 64, 0],
                          "display": os.environ.get("DISPLAY"), "wayland": os.environ.get("WAYLAND_DISPLAY")}
                pathlib.Path(args[args.index("--smoke-report") + 1]).write_text(json.dumps(report))
            '''))
            binary.chmod(0o755)
            result = subprocess.run(["bash", str(SMOKE)], cwd=ROOT, env={**os.environ, "BIN": folder, "RUNNER_TEMP": folder, "RUNNER_OS": "Linux", "DISPLAY": ":invalid", "WAYLAND_DISPLAY": "invalid"}, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = json.loads((root / "smoke-test.json").read_text())
            self.assertEqual(report["frames"], 1500)
            self.assertRegex(report["display"], r"^:[0-9]+$")
            self.assertIsNone(report["wayland"])


if __name__ == "__main__":
    unittest.main()
