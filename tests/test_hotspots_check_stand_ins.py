from pathlib import Path
import tempfile
import unittest

from tools.hotspots import check_stand_ins as check

HOOK = '''pub trait Hooks {
    /// A doc comment that names `Doc` and "Quoted" types is not code.
    #[hook(side = "server", contract = "listen", neoforge = "X.y")]
    fn placed(stack: &ItemStack, face: Face, pos: [i32; 3], map: Option<Vec<Known>>) -> Reply;
}
'''

VOCAB_TRAITS = '''pub trait Providers {
    fn provided(&self, name: &str) -> bool;
    fn with_default(&self) -> u32 {
        7
    }
    type Output;
    const LIMIT: u32 = 3;
}
#[horizon_mod_api_macros::host_area]
pub trait ToolsHost {
    fn tool(&self) -> u32;
}
'''


class StandInsTest(unittest.TestCase):
    def setUp(self):
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.root = Path(self._dir.name)
        self.write(f"{check.FIXTURES}/shared.rs", "//! Shared.\nmod stubs {\n    pub(super) use super::Api;\n}\n")
        self.write(f"{check.FIXTURES}/api/ItemStack.rs", "pub struct ItemStack;\nimpl ItemStack {\n    fn n(&self) {}\n}\n")
        self.write(f"{check.FIXTURES}/stubs/Known.rs", "pub(super) struct Known;\n")
        self.write(f"{check.FIXTURES}/stubs/Reply.rs", "pub(super) use crate::tests::mod_api::Reply;\n"
                   "pub(super) enum Reply {\n    A,\n}\n")
        self.write(f"{check.FIXTURES}/placed.rs", "mod api {\n    pub struct Face;\n}\n")
        self.write(f"{check.HOOKS}/placed.rs", HOOK)
        self.write(f"{check.VOCAB}/parts/tools.rs", VOCAB_TRAITS + "pub trait ToolsCapabilityProviders {}\n")
        self.write(f"{check.VOCAB}/parts/mod.rs", "pub trait CapabilityProviders {}\n")
        self.write(check.LOOPBACK, '''#[test]
fn table() {
    struct Table;
    impl CapabilityProviders for Table {}
    impl ToolsCapabilityProviders for Table {}
    impl mod_api::Providers for Table {
        type Output = ();
        fn provided(&self, _: &str) -> bool {
            let _ = "fn ignored(&self);";
            true
        }
    }
    impl mod_api::ToolsHost for Table {}
}
''')

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_a_clean_tree_passes(self):
        self.assertEqual(check.problems(self.root), [])

    def test_a_stand_in_file_defines_the_item_it_is_named_after(self):
        self.write(f"{check.FIXTURES}/stubs/Known.rs", "pub(super) struct Known;\npub(super) struct Extra;\n")
        self.write(f"{check.FIXTURES}/api/Wrong.rs", "pub struct Right;\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 2, found)
        self.assertIn("stubs/Known.rs: a stand-in file defines exactly the item it is named after, `Known`; it "
                      "defines `Known`, `Extra`", found[0])
        self.assertIn("api/Wrong.rs", found[1])

    def test_a_name_lives_in_one_folder(self):
        self.write(f"{check.FIXTURES}/api/Known.rs", "pub struct Known;\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("`Known` has a stand-in in both", found[0])

    def test_shared_rs_keeps_no_stand_ins(self):
        self.write(f"{check.FIXTURES}/shared.rs",
                   "//! Shared.\nmod stubs {\n    pub(super) use super::Api;\n    pub(super) struct Back;\n}\n"
                   "mod api {\n    pub struct A;\n}\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 2, found)
        self.assertIn("`mod stubs` defines `Back`; only `use` lines stay there. Move it to "
                      f"{check.FIXTURES}/stubs/Back.rs", found[0])
        self.assertIn("`mod api` is back", found[1])

    def test_a_hook_signature_type_needs_a_stand_in(self):
        (self.root / f"{check.FIXTURES}/placed.rs").unlink()
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("hook `placed` names type `Face`, which has no dispatch-test stand-in. Add "
                      f"{check.FIXTURES}/api/Face.rs", found[0])

    def test_a_loopback_impl_lacks_a_required_item(self):
        self.write(f"{check.VOCAB}/parts/tools.rs", VOCAB_TRAITS.replace(
            "    type Output;", "    type Output;\n    fn added(&self) -> Vec<String>;")
            + "pub trait ToolsCapabilityProviders {}\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn(f"{check.LOOPBACK}:6: `impl Providers for Table` lacks `added`, required by "
                      f"{check.VOCAB}/parts/tools.rs: add it to that impl in {check.LOOPBACK}", found[0])

    def test_a_union_needs_every_member_implemented(self):
        self.write(f"{check.VOCAB}/parts/more.rs", "pub trait MoreCapabilityProviders {\n    fn more(&self);\n}\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("`Table` implements the union `CapabilityProviders`, so it needs "
                      "`impl MoreCapabilityProviders for Table`", found[0])

    def test_child_files_of_the_loopback_are_read(self):
        self.write(check.LOOPBACK.replace(".rs", "/child.rs"),
                   "struct Other;\nimpl Providers for Other {\n    type Output = ();\n}\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("child.rs:2: `impl Providers for Other` lacks `provided`", found[0])


    def test_a_missing_input_path_fails_naming_it(self):
        (self.root / check.LOOPBACK).unlink()
        found = check.problems(self.root)
        self.assertEqual(found, [f"{check.LOOPBACK}: missing; tools/hotspots/check_stand_ins.py reads it. If it "
                                 "moved, update the script's path."])
        self.assertEqual(check.problems(self.root, "old/wire_loopback.rs")[0].split(":")[0], "old/wire_loopback.rs")

    def test_a_missing_shared_rs_fails_naming_it(self):
        (self.root / f"{check.FIXTURES}/shared.rs").unlink()
        self.assertEqual(check.problems(self.root), [f"{check.FIXTURES}/shared.rs: missing; "
                                                     "tools/hotspots/check_stand_ins.py reads it. If it moved, "
                                                     "update the script's path."])

    def test_no_hooks_or_no_union_members_fail_instead_of_passing(self):
        self.write(f"{check.HOOKS}/placed.rs", "pub trait Hooks {}\n")
        self.write(f"{check.VOCAB}/parts/tools.rs", VOCAB_TRAITS)
        found = check.problems(self.root)
        self.assertEqual(len(found), 2, found)
        self.assertIn("no `#[hook(...)] fn` found", found[0])
        self.assertIn("no `pub trait *CapabilityProviders` found", found[1])

    def test_union_members_may_be_implemented_in_another_loopback_file(self):
        self.write(check.LOOPBACK, (self.root / check.LOOPBACK).read_text().replace(
            "    impl ToolsCapabilityProviders for Table {}\n", ""))
        self.assertEqual(len(check.problems(self.root)), 1)
        self.write(check.LOOPBACK.replace(".rs", "/tools.rs"), "impl ToolsCapabilityProviders for Table {}\n")
        self.assertEqual(check.problems(self.root), [])

    def test_a_cfg_gated_required_item_is_never_required(self):
        self.write(f"{check.VOCAB}/parts/tools.rs", VOCAB_TRAITS.replace(
            "    type Output;", "    type Output;\n    #[cfg(any())]\n    fn never(&self);")
            + "pub trait ToolsCapabilityProviders {}\n")
        self.assertEqual(check.problems(self.root), [])


if __name__ == "__main__":
    unittest.main()
