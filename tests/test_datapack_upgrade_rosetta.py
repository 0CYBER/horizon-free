"""Jar extraction must stay inside the scratch folder.

`tools/datapack_upgrade_rosetta.py` promises that 'Nothing is written outside
a temporary folder'. A crafted or corrupted member named with ``..``, an
absolute path or Windows-style backslashes must be rejected rather than
clobbering files beside the scratch directory, while ordinary jar members are
still extracted unchanged.
"""
import importlib.util
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / "tools"
_spec = importlib.util.spec_from_file_location(
    "datapack_upgrade_rosetta", TOOLS / "datapack_upgrade_rosetta.py")
rosetta = importlib.util.module_from_spec(_spec)
_original_path = sys.path[:]
try:
    _spec.loader.exec_module(rosetta)
finally:
    # The tool inserts tools/ at sys.path[0] at import time; discovery must not inherit that.
    sys.path[:] = _original_path

LEGIT = "data/minecraft/worldgen/noise/legit.json"
TRAVERSAL = "data/minecraft/worldgen/../../../../ESCAPED.json"


def _jar(folder, members):
    jar = folder / "in.jar"
    with zipfile.ZipFile(jar, "w") as z:
        for name, payload in members:
            z.writestr(name, payload)
    return jar


class ExtractSafetyTest(unittest.TestCase):
    def test_traversal_member_does_not_write_outside_out(self):
        with tempfile.TemporaryDirectory() as d:
            base = Path(d)
            jar = _jar(base, [(TRAVERSAL, '{"pwned":true}'), (LEGIT, '{"ok":true}')])
            out = base / "out"
            out.mkdir()
            escaped = base / "ESCAPED.json"
            escaped.write_text('{"safe":"original"}')

            rosetta.extract(jar, out)

            self.assertEqual(escaped.read_text(), '{"safe":"original"}')
            self.assertEqual(
                sorted(p.relative_to(out).as_posix() for p in out.rglob("*") if p.is_file()),
                [LEGIT],
            )
            self.assertEqual((out / LEGIT).read_text(), '{"ok":true}')

    def test_normal_extraction_is_unchanged(self):
        with tempfile.TemporaryDirectory() as d:
            base = Path(d)
            names = [
                LEGIT,
                "data/minecraft/worldgen/noise/other.json",
                "data/minecraft/dimension_type/overworld.json",
            ]
            jar = _jar(base, [(n, '{"n":%d}' % i) for i, n in enumerate(names)])
            out = base / "out"
            out.mkdir()

            rosetta.extract(jar, out)

            for i, n in enumerate(names):
                self.assertEqual((out / n).read_text(), '{"n":%d}' % i)
            self.assertEqual(
                sorted(p.relative_to(out).as_posix() for p in out.rglob("*") if p.is_file()),
                sorted(names),
            )

    def test_backslash_traversal_member_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            base = Path(d)
            backslash = "data/minecraft/worldgen/..\\..\\..\\..\\ESCAPED_BS.json"
            jar = _jar(base, [(backslash, '{"pwned":true}'), (LEGIT, '{"ok":true}')])
            out = base / "out"
            out.mkdir()

            rosetta.extract(jar, out)

            self.assertFalse((base / "ESCAPED_BS.json").exists())
            self.assertTrue((out / LEGIT).is_file())

    def test_unsafe_names_are_rejected_by_safe_target(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "out"
            out.mkdir()
            for name in [
                "/absolute/path.json",
                "C:\\worldgen\\evil.json",
                "data/minecraft/worldgen/../../../../etc.json",
                "data/minecraft/worldgen/..\\..\\..\\..\\etc.json",
            ]:
                self.assertIsNone(rosetta._safe_target(out, name), name)
            self.assertIsNotNone(rosetta._safe_target(out, LEGIT))


if __name__ == "__main__":
    unittest.main()
