"""Unit tests for tools/test_boat_void_parity.py."""

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"


def _load_boat_void_module():
    spec = importlib.util.spec_from_file_location(
        "_boat_void_parity_under_test", TOOLS / "test_boat_void_parity.py"
    )
    module = importlib.util.module_from_spec(spec)
    inserted = str(TOOLS) not in sys.path
    if inserted:
        sys.path.insert(0, str(TOOLS))
    try:
        spec.loader.exec_module(module)
    finally:
        if inserted and str(TOOLS) in sys.path:
            sys.path.remove(str(TOOLS))
    return module


class BoatVoidHarnessTests(unittest.TestCase):
    def test_main_enables_loaded_fixture_and_boat_void_group(self):
        harness = _load_boat_void_module()
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp) / "run"
            work.mkdir()
            record = json.dumps({"op": "cmd", "result": 1}) + "\n"
            (work / "vanilla.jsonl").write_text(record, encoding="utf-8")
            (work / "rust.jsonl").write_text(record, encoding="utf-8")

            seen_argv = []

            def fake_driver_main():
                seen_argv.extend(sys.argv[1:])

            argv = ["test_boat_void_parity.py", "--work-dir", str(work)]
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                harness.driver, "main", side_effect=fake_driver_main
            ):
                harness.main()

            self.assertIn("--loaded-fixture", seen_argv)
            self.assertIn("--boat-void-fixture", seen_argv)
            self.assertIn("--allow-gaps", seen_argv)


if __name__ == "__main__":
    unittest.main()
