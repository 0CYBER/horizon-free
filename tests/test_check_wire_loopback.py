from pathlib import Path
import tempfile
import unittest

from tools.hotspots import check_wire_loopback

ROOT = "use super::*;\n#[test]\nfn kept() {}\n"


class WireLoopbackRootTests(unittest.TestCase):
    def test_a_hand_written_module_line_is_refused_with_or_without_a_base(self):
        now = ROOT + '#[path = "wire_loopback/friction.rs"]\nmod friction;\n'
        for before in (None, ROOT):
            found = check_wire_loopback.problems(now, before)
            self.assertEqual(len(found), 1)
            self.assertIn(":4: hand-written module line; add crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback/"
                          "<topic>.rs; it is discovered", found[0])

    def test_the_root_may_not_gain_a_test(self):
        grown = ROOT + "#[test]\nfn appended() {}\n"
        self.assertEqual(check_wire_loopback.problems(ROOT, ROOT), [])
        self.assertEqual(check_wire_loopback.problems(ROOT, grown), [])
        found = check_wire_loopback.problems(grown, ROOT)
        self.assertEqual(len(found), 1)
        self.assertIn("2 #[test]s, 1 at the merge base; add crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback/",
                      found[0])
        self.assertEqual(check_wire_loopback.problems(grown, None), [])

    def test_a_file_at_the_old_place_is_refused_naming_the_new_folder(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tests = root / "crates/horizon-mod-wasm/tests"
            (tests / "support").mkdir(parents=True)
            (tests / "support/helpers.rs").write_text("")
            self.assertEqual(check_wire_loopback.retired(root), [])
            (tests / "wire_loopback").mkdir()
            (tests / "wire_loopback/ingredient_builtins.rs").write_text("")
            self.assertEqual(check_wire_loopback.retired(root), [
                "crates/horizon-mod-wasm/tests/wire_loopback/ingredient_builtins.rs: nothing compiles this; "
                "move it to crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback/<topic>.rs"])

    def test_hand_written_hooks_objects_are_refused_but_a_feature_fragment_is_not(self):
        fragment = "pub(super) struct ItemTossHooks;\nimpl ItemTossHooks {\n    pub(super) fn item_toss(&self) {}\n}\n"
        self.assertEqual(check_wire_loopback.hook_problems("f.rs", fragment, False), [])
        self.assertEqual(check_wire_loopback.hook_problems("f.rs", "impl Hooks for Native {}\n", False), [])
        for text in ("impl mod_api::Hooks for Mod {}\n", "impl Hooks for ItemTossHooks {}\n",
                     "fn __horizon_mod_hooks() -> Box<dyn Hooks> { todo!() }\n"):
            found = check_wire_loopback.hook_problems("f.rs", text, False)
            self.assertEqual(len(found), 1, text)
            self.assertIn("<Feature>Hooks", found[0])
        self.assertEqual(check_wire_loopback.hook_problems("r.rs", "impl Hooks for First {}\n", True), [])
        self.assertEqual(len(check_wire_loopback.hook_problems("r.rs", "impl Hooks for Mod {}\n", True)), 1)
        export = "fn __horizon_mod_hooks() -> Box<dyn Hooks> {\n    Box::new(Mod)\n}\n"
        self.assertEqual(check_wire_loopback.hook_problems("r.rs", export, True), [])


if __name__ == "__main__":
    unittest.main()
