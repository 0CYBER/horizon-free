"""Self-mapping prose must be caught even when only one side is backticked.

Pure Python, no fixtures: markdown code decoration is not part of an identifier,
so the backreference in the gate has to compare bare names.
"""
import unittest

from tools import check_rename_prose as gate


class SelfMappingBacktickTests(unittest.TestCase):
    def test_becomes_spellings(self):
        for text in ("Foo becomes Foo", "`Foo` becomes `Foo`",
                     "`Foo` becomes Foo", "Foo becomes `Foo`"):
            with self.subTest(text=text):
                self.assertTrue(gate.check(text), text)

    def test_arrow_spellings(self):
        for text in ("Foo -> Foo", "`Foo` -> `Foo`",
                     "`Foo` -> Foo", "Foo -> `Foo`"):
            with self.subTest(text=text):
                self.assertTrue(gate.check(text), text)

    def test_renamed_spellings(self):
        for text in ("renamed Foo to Foo", "renamed `Foo` to `Foo`",
                     "renamed `Foo` to Foo", "renamed Foo to `Foo`"):
            with self.subTest(text=text):
                self.assertTrue(gate.check(text), text)

    def test_pair_spellings(self):
        for text in ("`Foo` | `Foo`", "`Foo` | Foo", "Foo | `Foo`",
                     "rename `Foo` and `Foo`", "rename `Foo` and Foo"):
            with self.subTest(text=text):
                self.assertTrue(gate.check(text), text)

    def test_qualified_names(self):
        for text in ("Foo::Bar becomes Foo::Bar",
                     "`Foo::Bar` becomes Foo::Bar"):
            with self.subTest(text=text):
                self.assertTrue(gate.check(text), text)

    def test_unrelated_prose_is_accepted(self):
        for text in ("Foo becomes Bar", "`Foo` becomes `Bar`",
                     "Foo -> Bar", "renamed Foo to Bar",
                     "renamed `Foo` to `Bar`", "Foo | Bar",
                     "Foo becomes FooImpl", "FooBar becomes Foo",
                     "the Foo class was removed"):
            with self.subTest(text=text):
                self.assertEqual(gate.check(text), [], text)


if __name__ == '__main__':
    unittest.main()
