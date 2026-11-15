"""Fishing CI setup uses the existing producer and never publishes its payloads."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import test_fishing_rng_parity as fishing


class FishingReferenceSetupTests(unittest.TestCase):
    def test_reference_only_producer_supplies_both_paths_and_isolated_homes(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)

            def produce(command, **kwargs):
                self.assertEqual(Path(command[1]).name, "test_command_exec_parity.py")
                self.assertIn("--reference-only", command)
                self.assertEqual(command[command.index("--only") + 1], "scoreboard")
                self.assertNotIn("cargo", command)
                self.assertIn("-Dhorizon.fishingReference=true", kwargs["env"]["JAVA_TOOL_OPTIONS"])
                for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
                    self.assertTrue(Path(kwargs["env"][key]).is_relative_to(work))
                for name in ("fishing-reference.json", "fishing-casts-reference.json"):
                    (work / name).write_text('[{"synthetic_setup_case": 1}]')
                return subprocess.CompletedProcess(command, 0)

            with patch.object(fishing.subprocess, "run", side_effect=produce):
                environment = fishing.prepare_reference(work)
            self.assertEqual(environment, {
                "HORIZON_FISHING_RESOURCES": str(fishing.lab.RESOURCES),
                "HORIZON_FISHING_REFERENCE": str(work / "fishing-reference.json"),
                "HORIZON_FISHING_CAST_REFERENCE": str(work / "fishing-casts-reference.json"),
            })

    def test_a_failed_producer_keeps_payloads_only_in_its_private_log(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)

            def fail(command, **kwargs):
                kwargs["stdout"].write("private oracle payload\n")
                return subprocess.CompletedProcess(command, 2)

            with patch.object(fishing.subprocess, "run", side_effect=fail):
                with self.assertRaises(SystemExit) as failure:
                    fishing.prepare_reference(work)
            self.assertNotIn("private oracle payload", str(failure.exception))
            self.assertIn("failed (2)", str(failure.exception))
            self.assertEqual((work / "prepare-reference.log").read_text(), "private oracle payload\n")

    def test_stale_inputs_cannot_stand_in_for_a_producer_that_wrote_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for name in ("fishing-reference.json", "fishing-casts-reference.json"):
                (work / name).write_text("stale")
            with patch.object(fishing.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)):
                with self.assertRaisesRegex(SystemExit, "did not write both required inputs"):
                    fishing.prepare_reference(work)
            self.assertFalse((work / "fishing-reference.json").exists())
            self.assertFalse((work / "fishing-casts-reference.json").exists())


if __name__ == "__main__":
    unittest.main()
