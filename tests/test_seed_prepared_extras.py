"""Regression: seeding must never write through a symlinked destination."""
import shutil
import tempfile
import unittest
from pathlib import Path

from tools.ci.seed_prepared_extras import seed


REL = Path("assets/minecraft/sounds/s.ogg")


class SeedPreparedExtrasTest(unittest.TestCase):
    def _tree(self, scratch, with_target_file=b"OLD"):
        root = Path(scratch)
        source, target = root / "src", root / "dst"
        (source / REL.parent).mkdir(parents=True)
        (source / REL).write_bytes(b"NEW")
        (target / REL.parent).mkdir(parents=True)
        if with_target_file is not None:
            (target / REL).write_bytes(with_target_file)
        return root, source, target

    def test_dangling_symlink_destination_is_refused(self):
        """A dangling symlink looks absent to exists() but must never be written through."""
        with tempfile.TemporaryDirectory() as scratch:
            root, source, target = self._tree(scratch, with_target_file=None)
            outside = root / "private" / "s.ogg"
            outside.parent.mkdir()
            link = target / REL
            link.symlink_to(outside)

            self.assertTrue(link.is_symlink())
            self.assertFalse(link.exists())  # dangling: exists() follows the link
            with self.assertRaisesRegex(ValueError, "symlink destination"):
                seed(source, target)
            self.assertFalse(outside.exists(), "file was created outside the target tree")
            self.assertFalse(link.exists(), "symlink target must stay untouched")

    def test_live_symlink_destination_is_refused(self):
        """Even a symlink to an existing file must not be overwritten."""
        with tempfile.TemporaryDirectory() as scratch:
            root, source, target = self._tree(scratch, with_target_file=None)
            outside = root / "private" / "s.ogg"
            outside.parent.mkdir()
            outside.write_bytes(b"SECRET")
            link = target / REL
            link.symlink_to(outside)

            with self.assertRaisesRegex(ValueError, "symlink destination"):
                seed(source, target)
            self.assertEqual(outside.read_bytes(), b"SECRET")

    def test_symlinked_parent_directory_is_refused(self):
        """A symlinked intermediate directory (e.g. target/assets) must not be written through: the leaf is a
        regular path, but mkdir/copyfile would still escape the target tree through the linked parent."""
        with tempfile.TemporaryDirectory() as scratch:
            root, source, target = self._tree(scratch, with_target_file=None)
            outside = root / "private" / "sounds"
            outside.mkdir(parents=True)
            # Replace target/assets (created by _tree) with a symlink to an outside dir.
            shutil.rmtree(target / "assets")
            link = target / "assets"
            link.symlink_to(outside, target_is_directory=True)

            self.assertTrue(link.is_symlink())
            with self.assertRaisesRegex(ValueError, "symlink destination"):
                seed(source, target)
            self.assertFalse((outside / "minecraft/sounds/s.ogg").exists(),
                             "file was created outside the target tree through a linked parent")

    def test_symlinked_parent_directory_pointing_inside_target_is_refused(self):
        """Any symlinked parent is refused, even one that happens to point back inside the target tree."""
        with tempfile.TemporaryDirectory() as scratch:
            root, source, target = self._tree(scratch, with_target_file=None)
            inside = target / "elsewhere" / "sounds"
            inside.mkdir(parents=True)
            shutil.rmtree(target / "assets")
            (target / "assets").symlink_to(inside, target_is_directory=True)

            with self.assertRaisesRegex(ValueError, "symlink destination"):
                seed(source, target)
            self.assertFalse((inside / "minecraft/sounds/s.ogg").exists())

    def test_regular_destination_is_seeded_unchanged(self):
        """Normal copies still work: a missing launcher extra is created as before."""
        with tempfile.TemporaryDirectory() as scratch:
            _, source, target = self._tree(scratch, with_target_file=None)
            seed(source, target)
            self.assertEqual((target / REL).read_bytes(), b"NEW")

    def test_existing_regular_destination_wins(self):
        """An existing non-symlink file is still skipped, not refused with an error."""
        with tempfile.TemporaryDirectory() as scratch:
            _, source, target = self._tree(scratch, with_target_file=b"OLD")
            seed(source, target)
            self.assertEqual((target / REL).read_bytes(), b"OLD")

    def test_non_extra_symlink_destination_is_ignored(self):
        """A symlink at a non-launcher-extra path is not our business and is skipped."""
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            source, target = root / "src", root / "dst"
            rel = Path("generated.json")
            (source / rel.parent).mkdir(parents=True, exist_ok=True)
            (source / rel).write_bytes(b"NEW")
            (target / rel.parent).mkdir(parents=True, exist_ok=True)
            outside = root / "private.json"
            outside.parent.mkdir(exist_ok=True)
            (target / rel).symlink_to(outside)

            seed(source, target)  # no error for non-extra paths
            self.assertFalse(outside.exists())


if __name__ == "__main__":
    unittest.main()
