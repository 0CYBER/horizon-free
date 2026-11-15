"""One real positive stage proof, run before documentation checks in verify: the staging pass over a clean
checkout writes the SDK's build data, each file with the header naming HEAD's commit.

Kept outside tests/ discovery so the later offline suite cannot stage twice.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("prebuilt_game", ROOT / "tools/prebuilt_game.py")
PREBUILT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREBUILT)


class StageBuildData(unittest.TestCase):
    def test_actual_stage_from_clean_checkout(self):
        self.assertTrue(shutil.which("cargo") and shutil.which("rustc"), "real stage requires Rust")
        subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=ROOT, check=True)
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        (ROOT / "target").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as temporary:
            stage_tool = os.environ.get("HORIZON_STAGE_TOOL")
            staged = PREBUILT.stage(Path(temporary), stage_tool=Path(stage_tool) if stage_tool else None)
            documents = {name: json.loads((staged / name).read_text()) for name in PREBUILT.BUILD_DATA}
            for name, document in documents.items():
                self.assertEqual(document["schema"], PREBUILT.BUILD_DATA_SCHEMA, name)
                self.assertEqual(document["build"]["commit"], commit, name)
            points = documents[PREBUILT.POINTS]["points"]
            self.assertTrue(points)
            self.assertTrue(all(len(point["signature_hash"]) == 32 for point in points))
            self.assertTrue(documents[PREBUILT.INTERNALS]["internals"])
            self.assertFalse((ROOT / ".horizon-source-staged").exists())
        # Staging must not change the tracked source it describes.
        subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=ROOT, check=True)


if __name__ == "__main__":
    unittest.main()
