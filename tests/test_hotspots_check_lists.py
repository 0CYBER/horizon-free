from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools.hotspots import check_lists


def names(n):
    return ", ".join(f"N{i}" for i in range(n))


class CheckListsTest(unittest.TestCase):
    def setUp(self):
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.root = Path(self._dir.name)
        self.folder = self.root / check_lists.EXPORTS
        self.folder.mkdir(parents=True)

    def write(self, name, text):
        (self.folder / name).write_text(text)

    def problems(self, long_lists):
        with mock.patch.dict(check_lists.LONG_LISTS, long_lists, clear=True):
            return check_lists.problems(self.root)

    def test_a_new_list_over_twelve_names_fails(self):
        self.write("short.rs", f"pub use a::{{{names(12)}}};\npub use b::One;\n")
        self.write("nested.rs", f"mod __private {{\n    pub(crate) use a::{{x::{{{names(7)}}}, {names(6)}}};\n}}\n")
        found = self.problems({})
        self.assertEqual(len(found), 1)
        self.assertIn("nested.rs: a `pub use` names 13 items", found[0])

    def test_an_entry_ends_with_its_list_and_a_pending_one_has_no_ceiling(self):
        self.write("fixed.rs", f"pub use a::{{{names(12)}}};\n")
        self.write("pending.rs", f"pub use a::{{{names(70)}}};\n")
        found = self.problems({"fixed.rs": (20, ""), "pending.rs": (None, "")})
        self.assertEqual(len(found), 1)
        self.assertIn("fixed.rs: no `pub use` list there names more than 12 items any more: remove its entry", found[0])

    def test_a_listed_length_may_only_shrink(self):
        self.write("grew.rs", f"pub use a::{{{names(15)}}};\n")
        self.write("shrank.rs", f"pub use a::{{{names(13)}}};\n")
        self.write("same.rs", f"pub use a::{{{names(20)}}};\n// pub use b::{{{names(30)}}};\n")
        found = self.problems({"grew.rs": (14, ""), "shrank.rs": (14, ""), "same.rs": (20, "")})
        self.assertEqual(len(found), 1)
        self.assertIn("grew.rs: its longest `pub use` list grew to 15 names (at most 14", found[0])

    def test_a_retired_fragment_that_comes_back_names_where_its_names_live(self):
        self.write("part_table.rs", "mod parts;\n")
        found = self.problems({})
        self.assertEqual(len(found), 1)
        self.assertIn("part_table.rs was removed", found[0])
        self.assertIn("exports/parts_<file>.rs", found[0])


if __name__ == "__main__":
    unittest.main()
