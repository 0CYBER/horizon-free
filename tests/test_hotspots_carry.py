import unittest

from hotspots_repo import Repo
from tools.hotspots import carry

OLD = "tools/ci/ignored-tests-known-failures.txt"
HEADER = "# PACKAGE TEST REFERENCE [policy]\n\n"


class DiffReadingTest(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.close)

    def test_rows_added_to_a_file(self):
        r = self.repo
        base = r.commit("main", {"list.txt": "a 1\nb 2\nc 3\n"})
        head = r.commit("pr", {"list.txt": "a 1\nb 2 changed\nc 3\nd 4\n"}, base="main")
        found = [(h.removed, h.added) for h in carry.hunks(r.path, base, head, "list.txt")]
        self.assertEqual(found, [(["b 2"], ["b 2 changed"]), ([], ["d 4"])])


class IgnoredTestCarryTest(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.close)
        r = self.repo
        self.base = r.commit("main", {OLD: HEADER + "pkg a::flaky_one #1 flaky\npkg a::gone #2\n"})
        # The switch on main: the old file is gone, its rows live in topic files.
        r.git("rm", "-q", OLD)
        r.commit("main", {"tools/ci/ignored-tests/flaky.txt": "pkg a::flaky_one #1 flaky\n",
                          "tools/ci/ignored-tests/known.txt": "pkg a::gone #2\n"}, "switch")
        self.folder = r.path / "tools/ci/ignored-tests"

    def pr(self, rows):
        return self.repo.commit("pr", {OLD: HEADER + rows}, base=self.base)

    def test_new_rows_go_to_the_topic_file_and_a_changed_row_changes_where_it_lives(self):
        head = self.pr("pkg a::flaky_one #9 flaky\npkg a::gone #2\n# Needs the new fixture.\npkg b::new #7 gpu\n")
        self.repo.git("checkout", "-q", "main")
        done = carry.carry_ignored_test(self.repo.path, self.base, head, "new-fixture")
        self.assertEqual(sorted(done), ["tools/ci/ignored-tests/flaky.txt", "tools/ci/ignored-tests/new-fixture.txt"])
        self.assertEqual((self.folder / "new-fixture.txt").read_text(), "# Needs the new fixture.\npkg b::new #7 gpu\n")
        self.assertEqual((self.folder / "flaky.txt").read_text(), "pkg a::flaky_one #9 flaky\n")
        self.assertEqual((self.folder / "known.txt").read_text(), "pkg a::gone #2\n")

    def test_a_hunk_it_cannot_place_stops_before_writing(self):
        r = self.repo
        r.git("checkout", "-q", "main")
        r.git("rm", "-q", "tools/ci/ignored-tests/known.txt")
        r.commit("main", {"tools/ci/ignored-tests/flaky.txt": "pkg a::flaky_one #1 flaky\n"}, "gone passes")
        # The PR replaces a::gone, which no topic file lists any more, with c::added.
        head = self.pr("pkg a::flaky_one #1 flaky\npkg c::added #3\n")
        r.git("checkout", "-q", "main")
        with self.assertRaises(carry.CarryError) as e:
            carry.carry_ignored_test(r.path, self.base, head, "added")
        self.assertIn("-pkg a::gone #2", str(e.exception))
        self.assertFalse((self.folder / "added.txt").exists())

    def test_a_row_moved_between_hunks_is_printed_not_dropped(self):
        r = self.repo
        old = HEADER + "pkg a::flaky_one #1 flaky\n" + "".join(f"pkg m::t{i} #5\n" for i in range(8)) + "pkg a::gone #2\n"
        base = r.commit("old", {OLD: old}, base=self.base)
        head = r.commit("old", {OLD: old.replace("pkg a::flaky_one #1 flaky\n", "").replace(
            "pkg a::gone #2\n", "pkg a::gone #2\npkg a::flaky_one #1 flaky\n")})
        r.git("checkout", "-q", "main")
        with self.assertRaises(carry.CarryError) as e:
            carry.carry_ignored_test(r.path, base, head, "moved")
        self.assertIn("moves a row between hunks", str(e.exception))
        self.assertIn("+pkg a::flaky_one #1 flaky", str(e.exception))



PARTS = "crates/horizon-mod-vocab/src/parts"
EXPORTS = "crates/horizon-mod-vocab/src/exports"
TABLE = """mod parts;
pub use parts::{Attached, BlockProvider};
mod __private {
    pub use crate::parts::{AttributeDeclaration, attribute_declarations};
}
"""


class PartTableCarryTest(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.close)
        r = self.repo
        self.parts = {f"{PARTS}/mod.rs": "include!(\"x\");\n",
                      f"{PARTS}/unions.rs.in": "pub trait Registrar {}\n",
                      f"{PARTS}/shared.rs": "pub struct Attached;\nimpl Attached {\n    pub fn not_an_item() {}\n}\n",
                      f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\n",
                      f"{PARTS}/attributes.rs": "pub struct AttributeDeclaration;\n"
                                                "pub fn attribute_declarations() {}\n"}
        self.base = r.commit("main", {**self.parts, f"{EXPORTS}/part_table.rs": TABLE})
        # The switch on main, with per-source export fragments.
        r.git("rm", "-q", f"{EXPORTS}/part_table.rs")
        r.commit("main", {
            f"{EXPORTS}/parts.rs": "mod parts;\n",
            f"{EXPORTS}/parts_shared.rs": "pub use parts::Attached;\n",
            f"{EXPORTS}/parts_blocks.rs": "pub use parts::BlockProvider;\n",
            f"{EXPORTS}/parts_attributes.rs": "mod __private {\n"
                                              "    pub use crate::parts::{AttributeDeclaration, attribute_declarations};\n"
                                              "}\n"}, "switch")

    def pr(self, table, parts):
        head = self.repo.commit("pr", {**parts, f"{EXPORTS}/part_table.rs": table}, base=self.base)
        self.repo.git("checkout", "-q", "main")
        return head

    def read(self, name):
        return (self.repo.path / EXPORTS / name).read_text()

    def test_added_names_go_to_their_files_fragments_in_their_scope(self):
        table = TABLE.replace("Attached, BlockProvider", "Attached, BlockProvider, BlockCount").replace(
            "attribute_declarations}", "attribute_declarations, AttributeBoundsWidening, attribute_bounds_widenings}")
        table = table.replace("mod __private {\n", "mod __private {\n    #[cfg(test)]\n"
                                                    "    pub use crate::parts::tests as t;\n")
        head = self.pr(table, {
            f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\npub const BlockCount: u8 = 1;\n",
            f"{PARTS}/attribute_bounds.rs": "pub struct AttributeBoundsWidening;\n"
                                            "pub(crate) fn attribute_bounds_widenings() {}\n"})
        done = carry.carry_part_table(self.repo.path, self.base, head, None)
        self.assertEqual(sorted(done), [f"{EXPORTS}/parts.rs", f"{EXPORTS}/parts_attribute_bounds.rs",
                                        f"{EXPORTS}/parts_blocks.rs"])
        self.assertEqual(self.read("parts_blocks.rs"), "pub use parts::BlockProvider;\npub use parts::BlockCount;\n")
        self.assertIn("mod __private {\n    pub use crate::parts::AttributeBoundsWidening;\n"
                      "    pub use crate::parts::attribute_bounds_widenings;\n}\n",
                      self.read("parts_attribute_bounds.rs"))
        self.assertEqual(self.read("parts.rs"), "mod parts;\nmod __private {\n    #[cfg(test)]\n"
                                                "    pub use crate::parts::tests as t;\n}\n")

    def test_a_name_no_parts_file_defines_stops_before_writing(self):
        table = TABLE.replace("Attached, BlockProvider", "Attached, BlockProvider, BlockCount, Ghost")
        head = self.pr(table, {f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\npub fn BlockCount() {}\n"})
        with self.assertRaises(carry.CarryError) as e:
            carry.carry_part_table(self.repo.path, self.base, head, None)
        self.assertIn("Ghost (root scope)", str(e.exception))
        self.assertNotIn("BlockCount", self.read("parts_blocks.rs"))

    def test_a_name_main_already_exports_is_not_written_twice(self):
        r = self.repo
        r.commit("main", {f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\npub fn block_count() {}\n",
                          f"{EXPORTS}/parts_blocks.rs": "pub use parts::{BlockProvider, block_count};\n"}, "landed")
        head = self.pr(TABLE.replace("Attached, BlockProvider", "Attached, BlockProvider, block_count"),
                       {f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\npub fn block_count() {}\n"})
        self.assertEqual(carry.carry_part_table(r.path, self.base, head, None), [])


    def assert_refused(self, head, *diagnostics):
        before = {p.name: p.read_text() for p in (self.repo.path / EXPORTS).glob("*.rs")}
        with self.assertRaises(carry.CarryError) as e:
            carry.carry_part_table(self.repo.path, self.base, head, None)
        for diagnostic in diagnostics:
            self.assertIn(diagnostic, str(e.exception))
        self.assertEqual({p.name: p.read_text() for p in (self.repo.path / EXPORTS).glob("*.rs")}, before)

    def test_a_name_main_exports_differently_stops_before_writing(self):
        r = self.repo
        r.commit("main", {f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\npub fn block_count() {}\n",
                          f"{EXPORTS}/parts_blocks.rs": "pub use parts::BlockProvider;\n#[cfg(test)]\n"
                                                        "pub(crate) use parts::block_count;\n"}, "landed")
        table = TABLE.replace("Attached, BlockProvider", "Attached, BlockProvider, block_count").replace(
            "attribute_declarations}", "attribute_declarations, attribute_count}")
        head = self.pr(table, {f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\npub fn block_count() {}\n",
                               f"{PARTS}/attributes.rs": "pub struct AttributeDeclaration;\n"
                                                         "pub fn attribute_declarations() {}\n"
                                                         "pub fn attribute_count() {}\n"})
        self.assert_refused(head, "block_count (root scope): this tree already exports that name with another cfg",
                            "pub use parts::block_count;")

    def test_a_removed_name_stops_before_writing(self):
        table = TABLE.replace("Attached, BlockProvider", "Attached, BlockCount")
        head = self.pr(table, {f"{PARTS}/blocks.rs": "pub fn BlockCount() {}\n"})
        self.assert_refused(head, "BlockProvider (root scope): the PR removed or changed this line of part_table.rs: "
                                  "pub use parts::BlockProvider;")

    def test_a_renamed_name_stops_before_writing(self):
        table = TABLE.replace("Attached, BlockProvider}", "Attached, BlockProvider, BlockCount}").replace(
            "mod __private {\n", "mod __private {\n    pub use crate::parts::KnownIds as Ids;\n")
        base_table = TABLE.replace("mod __private {\n", "mod __private {\n    pub use crate::parts::KnownIds as Known;\n")
        r = self.repo
        self.base = r.commit("old", {**self.parts, f"{PARTS}/shared.rs": self.parts[f"{PARTS}/shared.rs"] +
                                     "pub struct KnownIds;\n", f"{EXPORTS}/part_table.rs": base_table},
                             base=self.base)
        head = r.commit("old", {f"{EXPORTS}/part_table.rs": table,
                                f"{PARTS}/blocks.rs": "pub trait BlockProvider {}\npub fn BlockCount() {}\n"})
        r.git("checkout", "-q", "main")
        self.assert_refused(head, "Known (__private scope): the PR removed or changed this line of part_table.rs: "
                                  "pub use crate::parts::KnownIds as Known;")

STUBS = carry.check_stand_ins.FIXTURES
OLD_SHARED = """//! Shared.
fn helper() {}

mod stubs {
    pub(super) use super::Api;
    // Kept apart from the API's.
    #[derive(Clone, Copy)]
    pub(super) struct Pos;
    pub(super) struct View<'a> {
        pub(super) id: &'a str,
    }
    impl View<'_> {
        pub(super) fn id(&self) -> &str {
            self.id
        }
    }
    pub(super) struct Gone;
}

mod api {
    pub enum Face {
        Up,
    }
}
"""


class DispatchStubCarryTest(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.close)
        r = self.repo
        self.base = r.commit("main", {carry.SHARED: OLD_SHARED})
        # The switch on main, with per-name stand-in fixtures.
        found, rest = carry.stand_ins(OLD_SHARED)
        r.commit("main", {carry.SHARED: rest, **{f"{STUBS}/{scope}/{name}.rs": text
                                                 for (scope, name), text in found.items()}}, "switch")
        self.folder = r.path / STUBS

    def test_the_split_keeps_use_lines_and_gives_each_stand_in_its_impls_and_comments(self):
        found, rest = carry.stand_ins(OLD_SHARED)
        self.assertEqual(sorted(found), [("api", "Face"), ("stubs", "Gone"), ("stubs", "Pos"), ("stubs", "View")])
        self.assertEqual(found[("stubs", "Pos")], "// Kept apart from the API's.\n#[derive(Clone, Copy)]\n"
                                                   "pub(super) struct Pos;\n")
        self.assertIn("impl View<'_> {\n    pub(super) fn id", found[("stubs", "View")])
        self.assertEqual(rest, "//! Shared.\nfn helper() {}\n\nmod stubs {\n    pub(super) use super::Api;\n}\n")

    def test_added_changed_and_removed_stand_ins_reach_their_files(self):
        r = self.repo
        head = r.commit("pr", {carry.SHARED: OLD_SHARED.replace(
            "    pub(super) struct Gone;\n", "    pub(super) enum Entry {\n        Short,\n    }\n").replace(
            "            self.id\n        }\n", "            self.id\n        }\n        pub(super) fn begin(&mut self) {}\n")
            .replace("fn helper() {}", "fn helper() {}\nfn other() {}")}, base=self.base)
        r.git("checkout", "-q", "main")
        done = carry.carry_dispatch_stub(r.path, self.base, head, None)
        self.assertEqual(sorted(done), [f"{STUBS}/stubs/Entry.rs", f"{STUBS}/stubs/Gone.rs", f"{STUBS}/stubs/View.rs"])
        self.assertEqual((self.folder / "stubs/Entry.rs").read_text(), "pub(super) enum Entry {\n    Short,\n}\n")
        self.assertIn("pub(super) fn begin(&mut self) {}", (self.folder / "stubs/View.rs").read_text())
        self.assertFalse((self.folder / "stubs/Gone.rs").exists())
        self.assertTrue((self.folder / "stubs/Pos.rs").exists())

    def test_a_change_inside_a_string_literal_is_carried(self):
        r = self.repo
        head = r.commit("pr", {carry.SHARED: OLD_SHARED.replace("            self.id\n", "            \"a b\"\n")},
                        base=self.base)
        r.git("checkout", "-q", "main")
        self.assertEqual(carry.carry_dispatch_stub(r.path, self.base, head, None), [f"{STUBS}/stubs/View.rs"])
        view = f"{STUBS}/stubs/View.rs"
        r.commit("main", {view: (r.path / view).read_text()}, "carried")
        head2 = r.commit("pr2", {carry.SHARED: OLD_SHARED.replace("            self.id\n", "            \"ab\"\n")},
                         base=head)
        r.git("checkout", "-q", "main")
        self.assertEqual(carry.carry_dispatch_stub(r.path, head, head2, None), [f"{STUBS}/stubs/View.rs"])
        self.assertIn('"ab"', (self.folder / "stubs/View.rs").read_text())

    def test_a_stand_in_main_changed_too_stops_before_writing(self):
        r = self.repo
        r.commit("main", {f"{STUBS}/api/Face.rs": "pub enum Face {\n    Up,\n    Down,\n}\n"}, "main edit")
        head = r.commit("pr", {carry.SHARED: OLD_SHARED.replace("        Up,\n", "        Up,\n        North,\n")
                               .replace("pub(super) struct Pos;", "pub(super) struct Pos(u8);")}, base=self.base)
        r.git("checkout", "-q", "main")
        with self.assertRaises(carry.CarryError) as e:
            carry.carry_dispatch_stub(r.path, self.base, head, None)
        self.assertIn(f"{STUBS}/api/Face.rs on this tree differs from the PR's base", str(e.exception))
        self.assertIn("North,", str(e.exception))
        self.assertIn("pub(super) struct Pos;", (self.folder / "stubs/Pos.rs").read_text())


if __name__ == "__main__":
    unittest.main()
