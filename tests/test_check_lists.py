"""Regression for check_lists.longest_list() counting generic-argument commas."""
import unittest

from tools.hotspots import check_lists


class LongestListTest(unittest.TestCase):
    def test_generic_argument_comma_is_not_a_separate_name(self):
        self.assertEqual(check_lists.longest_list("pub use a::{Foo<Bar, Baz>, Qux};\n"), 2)

    def test_nested_generics_are_one_name_each(self):
        self.assertEqual(check_lists.longest_list("pub use a::{Foo<Bar<Baz, Qux>, Quux>, Corge};\n"), 2)

    def test_simple_and_nested_use_lists_are_unchanged(self):
        self.assertEqual(check_lists.longest_list("pub use a::{Foo, Bar, Baz};\n"), 3)
        self.assertEqual(check_lists.longest_list("pub use a::Foo;\n"), 0)
        self.assertEqual(check_lists.longest_list("pub use a::{x::{Foo, Bar}, Baz};\n"), 3)


if __name__ == "__main__":
    unittest.main()
