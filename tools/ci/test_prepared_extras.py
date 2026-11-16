"""Shared extras must not alter the exporter's canonical output or replace its files."""
from pathlib import Path
import tempfile
import unittest

from tools.ci.seed_prepared_extras import seed
from tools.prepare_output import digest

ROOT = Path(__file__).resolve().parents[2]


class PreparedExtrasTest(unittest.TestCase):
    def test_only_launcher_assets_are_seeded_and_existing_files_win(self):
        (ROOT / "target").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            source, target = [Path(directory) / name for name in ("source", "target")]
            target.mkdir()
            (target / "horizon-resources.json").write_text('{"schema": 7}')
            (target / "generated.json").write_text('{"value": "new"}')
            extras = {
                "assets/minecraft/sounds/fixture.ogg": b"sound",
                "assets/minecraft/sounds.json": b"{}",
                "assets/minecraft/lang/fr_fr.json": b"{}",
                "assets/pack.mcmeta": b"{}",
                "asset-index.json": b"{}",
            }
            refused = {
                "generated.json": b"old",
                "retired-export.json": b"{}",
                "assets/minecraft/lang/en_us.json": b"{}",
                "assets/minecraft/lang/deprecated.json": b"{}",
                "horizon-data.bin": b"old cache",
                "horizon-sprites.cache": b"old cache",
                "export.log": b"old log",
                "Exporter.class": b"old class",
            }
            for name, data in {**extras, **refused}.items():
                path = source / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            # Even an allowed file name cannot read through a shared symlink.
            (source / "assets/minecraft/sounds/link.ogg").symlink_to(source / "generated.json")
            existing = target / "assets/minecraft/sounds.json"
            existing.parent.mkdir(parents=True)
            existing.write_bytes(b'{"exported": true}')
            before = digest(target)
            seed(source, target)
            self.assertEqual(digest(target), before)
            self.assertEqual(existing.read_bytes(), b'{"exported": true}')
            self.assertEqual((target / "generated.json").read_text(), '{"value": "new"}')
            for name, data in extras.items():
                if name != "assets/minecraft/sounds.json":
                    self.assertEqual((target / name).read_bytes(), data)
            for name in refused:
                if name != "generated.json":
                    self.assertFalse((target / name).exists(), name)
            self.assertFalse((target / "assets/minecraft/sounds/link.ogg").exists())


if __name__ == "__main__":
    unittest.main()
