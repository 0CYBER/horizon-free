"""Exercise the workflow's scratch allocation without starting a build."""
import os
import importlib.util
from unittest.mock import patch
from pathlib import Path
import shutil
import subprocess
import tempfile
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "requires POSIX Bash")
class MixinsScratchTests(unittest.TestCase):
    def test_linkage_stage_and_outputs_are_job_owned(self):
        """Follow the real builder's stage derivation from isolated workflow env."""
        workflow = yaml.load((ROOT / ".github/workflows/native-dylib-probe.yml").read_text(), Loader=yaml.BaseLoader)
        job = workflow["jobs"]["linkage"]
        self.assertEqual(job["permissions"], {"contents": "read"})
        steps = {step.get("name"): step for step in job["steps"]}
        allocate = steps["Allocate private persistent linkage workspace"]["run"]
        cleanup = steps["Clean private linkage workspace"]["run"]
        spec = importlib.util.spec_from_file_location("scratch_prebuilt", ROOT / "tools/prebuilt_game.py")
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "persistent-target"
            receipt = root / "env"
            original = dict(os.environ, CARGO_TARGET_DIR=str(target), GITHUB_ENV=str(receipt))
            allocated = []
            for _ in range(2):
                receipt.write_text("")
                subprocess.run(["bash", "-eu", "-c", allocate], env=original, check=True)
                exports = dict(line.split("=", 1) for line in receipt.read_text().splitlines())
                scratch = Path(exports["LINKAGE_SCRATCH"])
                allocated.append(scratch)
                self.assertEqual(scratch.parent, target)
                self.assertEqual(scratch.stat().st_mode & 0o777, 0o700)
                for key in ("CARGO_TARGET_DIR", "SDK_OUT", "PROBE_OUT"):
                    self.assertTrue(Path(exports[key]).is_relative_to(scratch))
                with patch.dict(os.environ, exports), patch.object(builder, "stage", side_effect=RuntimeError("stage boundary")) as stage:
                    with self.assertRaisesRegex(RuntimeError, "stage boundary"):
                        builder.build(Path(exports["SDK_OUT"]), False)
                    stage.assert_called_once_with(Path(exports["CARGO_TARGET_DIR"]) / "prebuilt")
                subprocess.run(["bash", "-eu", "-c", cleanup], env=dict(original, **exports), check=True)
                self.assertFalse(scratch.exists())
            self.assertEqual(len(set(allocated)), 2)

    def test_self_hosted_scratch_is_private_unique_and_off_tmpfs(self):
        """Run the workflow's own allocation and cleanup steps twice."""
        workflow = yaml.load((ROOT / ".github/workflows/fn-replace-check.yml").read_text(), Loader=yaml.BaseLoader)
        steps = {step.get("name"): step for step in workflow["jobs"]["fn-replace-check"]["steps"]}
        allocate = steps["Allocate private scratch"]["run"]
        cleanup = steps["Clean private scratch"]
        self.assertEqual(cleanup["if"], "always()")
        for name, step in steps.items():
            if name not in ("Allocate private scratch", "Clean private scratch"):
                self.assertNotIn("RUNNER_TEMP", step.get("run", ""), name)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            persistent_target = root / "persistent-target"
            receipt = root / "env"
            env = dict(os.environ, CARGO_TARGET_DIR=str(persistent_target), RUNNER_TEMP=str(root / "tmpfs"),
                       RUNNER_ENVIRONMENT="self-hosted", GITHUB_ENV=str(receipt))
            paths = []
            for _ in range(2):
                receipt.write_text("")
                subprocess.run(["bash", "-eu", "-c", allocate], env=env, check=True)
                exports = dict(line.split("=", 1) for line in receipt.read_text().splitlines())
                scratch = Path(exports["FNR_SCRATCH"])
                paths.append(scratch)
                self.assertEqual(scratch.parent, persistent_target)
                self.assertFalse(scratch.is_relative_to(root / "tmpfs"))
                self.assertEqual(scratch.stat().st_mode & 0o777, 0o700)
                (scratch / "prebuilt").mkdir()
                subprocess.run(["bash", "-eu", "-c", cleanup["run"]], env=dict(env, **exports), check=True)
                self.assertFalse(scratch.exists(), "scratch is cleaned even on a persistent build volume")
            self.assertEqual(len(set(paths)), 2)
            receipt.write_text("")
            subprocess.run(["bash", "-eu", "-c", allocate], env=dict(env, RUNNER_ENVIRONMENT="github-hosted"), check=True)
            self.assertEqual(receipt.read_text().strip(), f"FNR_SCRATCH={root / 'tmpfs/fn-replace-check'}")


if __name__ == "__main__":
    unittest.main()
