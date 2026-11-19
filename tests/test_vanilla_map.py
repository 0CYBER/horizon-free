"""Tests for the vanilla-map tool's Rust comment extraction."""

import unittest

from tools import vanilla_map as vm


class CommentTextTest(unittest.TestCase):
    def test_slashes_in_a_string_are_not_a_comment(self):
        line = 'let url = "https://example.invalid//path"; // real comment'
        self.assertEqual(vm.comment_text(line), "// real comment")

    def test_slashes_in_a_raw_string_are_not_a_comment(self):
        line = 'let url = r#"http://net.minecraft.example/FooBar"#; // real comment'
        self.assertEqual(vm.comment_text(line), "// real comment")

    def test_escaped_quote_does_not_end_the_string(self):
        line = r'let s = "a\"//not-a-comment"; // real comment'
        self.assertEqual(vm.comment_text(line), "// real comment")

    def test_char_literal_slashes_are_not_a_comment(self):
        line = "let c = '/'; // real comment"
        self.assertEqual(vm.comment_text(line), "// real comment")

    def test_no_comment_returns_empty(self):
        self.assertEqual(vm.comment_text('let url = "https://example.invalid//path";'), "")

    def test_rust_references_ignores_a_class_inside_a_url_string(self):
        names = vm.Names(["net/minecraft/example/FooBar"])
        lines = 'let url = "http://net.minecraft.example/FooBar"; // plain comment\n'
        refs = vm.rust_references(["example.rs"], names, read=lambda f: lines)
        self.assertEqual(refs, {})


if __name__ == "__main__":
    unittest.main()
