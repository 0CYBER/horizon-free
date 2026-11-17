#!/usr/bin/env python3
"""CLI regression for --changed-files read errors in tools/parity.py."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class ChangedFilesCliTests(unittest.TestCase):
    """Test that parity.py handles missing/invalid --changed-files paths gracefully."""

    def test_changed_files_cli_errors(self):
        """SubTest: missing file and directory paths give exit 2, no traceback."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            invalid = root / "invalid.txt"
            invalid.write_bytes(b"\xff")
            test_cases = [
                ("missing", root / "missing.txt"),
                ("directory", root),
                ("invalid UTF-8", invalid),
            ]
            for name, path in test_cases:
                with self.subTest(name=name):
                    result = subprocess.run(
                        [sys.executable, str(ROOT / "tools" / "parity.py"),
                         "--changed-files", str(path), "--list"],
                        capture_output=True, text=True, timeout=30,
                        cwd=ROOT,
                    )
                    self.assertEqual(result.returncode, 2,
                                     f"{name}: expected exit 2, got {result.returncode}")
                    stderr = result.stderr
                    self.assertIn("error:", stderr,
                                  f"{name}: expected 'error:' in stderr, got: {stderr}")
                    self.assertNotIn("Traceback", stderr,
                                     f"{name}: no traceback allowed, got: {stderr}")
                    # The path should appear in the error message
                    self.assertIn(str(path), stderr,
                                  f"{name}: path {path} should appear in stderr")


if __name__ == "__main__":
    unittest.main()