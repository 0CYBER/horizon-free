"""A test asks for a Rust function by name, so moving it to another file changes nothing."""
from pathlib import Path
import tempfile
import unittest

from rust_items import rust_fn


class RustFnTests(unittest.TestCase):
    def test_finds_the_function_in_any_file_and_refuses_a_duplicate(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "a").mkdir()
            (root / "a/one.rs").write_text(
                "impl Game {\n    /// Docs.\n    pub(in crate::game) fn moved(&self) -> u8 {\n"
                "        if true { 1 } else { 2 }\n    }\n    fn movedx(&self) {}\n}\n")
            self.assertEqual(rust_fn("moved", under=root / "a"),
                             "    fn moved(&self) -> u8 {\n        if true { 1 } else { 2 }\n    }")
            (root / "b").mkdir()
            (root / "b/two.rs").write_text('pub const fn moved() {}\npub unsafe extern "C" fn native() {}\n')
            self.assertEqual(rust_fn("moved", under=root / "b"), "    const fn moved() {}")
            self.assertEqual(rust_fn("native", under=root / "b"), '    unsafe extern "C" fn native() {}')
            with self.assertRaises(LookupError):
                rust_fn("moved", under=root)
            with self.assertRaises(LookupError):
                rust_fn("absent", under=root)

    def test_an_owner_keeps_a_common_name_to_its_impl(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "game.rs").write_text("impl Game<'_> {\n    fn tick(&mut self) { a() }\n}\n")
            (root / "other.rs").write_text(
                "impl Display for Other {\n    fn tick(&self) { b() }\n}\nfn tick() {}\n")
            self.assertEqual(rust_fn("tick", under=root, owner="Game"), "    fn tick(&mut self) { a() }")
            self.assertEqual(rust_fn("tick", under=root, owner="Other"), "    fn tick(&self) { b() }")
            with self.assertRaises(LookupError):
                rust_fn("tick", under=root, owner="Missing")


if __name__ == "__main__":
    unittest.main()
