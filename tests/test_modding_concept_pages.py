"""Focused discovery, nesting and link-check regressions for the modding book."""
import copy
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

with patch.object(sys, "path", [*sys.path, str(Path(__file__).resolve().parents[1] / "tools")]):
    import modding_concept_pages as pages


class ConceptPages(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.src = Path(self.tmp.name)
        self.book = {"sections": []}
        for part, folder in pages.FOLDERS.items():
            (self.src / folder).mkdir()
            self.book["sections"].append({"PartTitle": part})
        (self.src / "concepts/parts.md").write_text("# Parts\n\n[block](parts/b-block.md#block-parts)\n")
        (self.src / "concepts/parts").mkdir()
        (self.src / "concepts/parts/b-block.md").write_text("# Block parts\n\n## Landing\n")
        (self.src / "concepts/parts/a-item.md").write_text("# Item parts\n")
        self.book["sections"].insert(1, {"Chapter": dict(name="Parts", path="concepts/parts.md",
            source_path="concepts/parts.md", content=(self.src / "concepts/parts.md").read_text(),
            number=[1], parent_names=[], sub_items=[])})

    def expand(self, book=None):
        return pages.expand(copy.deepcopy(book or self.book), self.src)

    def test_import_does_not_change_discovery_search_paths(self):
        spec = importlib.util.spec_from_file_location("concept_import_probe", __file__)
        with patch.object(sys, "path", list(sys.path)):
            before = list(sys.path)
            spec.loader.exec_module(importlib.util.module_from_spec(spec))
            self.assertEqual(sys.path, before)

    def test_a_folder_beside_a_page_gives_its_sub_chapters_in_file_name_order(self):
        parts = self.expand()["sections"][1]["Chapter"]
        self.assertEqual([c["Chapter"]["name"] for c in parts["sub_items"]], ["Item parts", "Block parts"])
        self.assertEqual([c["Chapter"]["number"] for c in parts["sub_items"]], [[1, 1], [1, 2]])
        self.assertEqual(parts["sub_items"][0]["Chapter"]["parent_names"], ["Parts"])

    def test_section_pages_marker_lists_the_sub_chapters(self):
        (self.src / "concepts/parts.md").write_text("# Parts\n\n<!-- section-pages -->\n")
        book = copy.deepcopy(self.book)
        book["sections"][1]["Chapter"]["content"] = (self.src / "concepts/parts.md").read_text()
        parts = self.expand(book)["sections"][1]["Chapter"]
        self.assertIn("- [Item parts](parts/a-item.md)\n- [Block parts](parts/b-block.md)", parts["content"])

    def test_nested_folders_nest_again(self):
        (self.src / "concepts/parts/a-item").mkdir()
        (self.src / "concepts/parts/a-item/tags.md").write_text("# Tag parts\n")
        item = self.expand()["sections"][1]["Chapter"]["sub_items"][0]["Chapter"]
        self.assertEqual([c["Chapter"]["path"] for c in item["sub_items"]], ["concepts/parts/a-item/tags.md"])

    def test_unlisted_page_joins_its_part_by_title_and_is_numbered(self):
        (self.src / "concepts/zeta.md").write_text("# Zeta\n\nLast by title.\n")
        (self.src / "concepts/alpha.md").write_text("# Alpha\n\n[block](parts/b-block.md#landing)\n")
        result = self.expand()
        flat = [(i.get("PartTitle"), i.get("Chapter", {}).get("name"), i.get("Chapter", {}).get("number"))
                for i in result["sections"]]
        self.assertEqual(flat, [("Concepts", None, None), (None, "Parts", [1]), (None, "Alpha", [2]),
                                (None, "Zeta", [3]), ("How-to", None, None)])

    def test_equal_titles_use_filename_order_not_directory_iteration_order(self):
        for name in ("zeta", "alpha"):
            (self.src / f"how-to/{name}.md").write_text("# Same title\n")
        glob = Path.glob

        def reversed_glob(path, *args, **kwargs):
            return iter(sorted(glob(path, *args, **kwargs), reverse=True))
        with patch.object(Path, "glob", reversed_glob):
            result = self.expand()
        found = [c["path"] for c in pages.chapters(result["sections"]) if c["path"].startswith("how-to/")]
        self.assertEqual(found, ["how-to/alpha.md", "how-to/zeta.md"])

    def test_design_page_dropped_into_concepts_fails(self):
        (self.src / "concepts/next.md").write_text("# Design: what comes next\n\nA proposal.\n")
        with self.assertRaisesRegex(AssertionError, "concepts/next.md: .* docs/design/"):
            self.expand()

    def test_design_page_dropped_into_a_group_folder_fails(self):
        (self.src / "concepts/parts/c-plan.md").write_text("# Parts roadmap\n")
        with self.assertRaisesRegex(AssertionError, "concepts/parts/c-plan.md: .* docs/design/"):
            self.expand()

    def test_page_outside_the_discovered_folders_must_be_listed(self):
        (self.src / "reference").mkdir()
        (self.src / "reference/orphan.md").write_text("# Orphan\n")
        with self.assertRaisesRegex(AssertionError, "doesn't render.*reference/orphan.md"):
            self.expand()

    def test_discovered_page_links_are_checked(self):
        (self.src / "how-to/broken.md").write_text("# Broken\n\n[gone](missing.md)\n")
        with self.assertRaisesRegex(AssertionError, "how-to/broken.md: missing link target"):
            self.expand()

    def test_missing_relative_target_or_anchor_fails(self):
        path = self.src / "concepts/parts/a-item.md"
        for target, error in [("missing.md", "missing link target"), ("b-block.md#absent", "missing anchor")]:
            path.write_text(f"# Item parts\n\n[x]({target})\n")
            with self.assertRaisesRegex(AssertionError, error):
                self.expand()

    def test_links_resolve_generated_chapter_and_anchor_without_a_file(self):
        book = copy.deepcopy(self.book)
        book["sections"].append({"Chapter": dict(name="Hook notes", path="reference/hook-notes.md",
            source_path=None, content="# Hook notes\n\n## Example\n", number=[9], parent_names=[], sub_items=[])})
        path = self.src / "concepts/parts/a-item.md"
        path.write_text("# Item parts\n\n[notes](../../reference/hook-notes.md#example)\n")
        self.expand(book)
        path.write_text(path.read_text().replace("#example", "#absent"))
        with self.assertRaisesRegex(AssertionError, "missing anchor"):
            self.expand(book)

    def test_reference_destinations_are_checked(self):
        for target, error in [("missing.md", "missing link target"), ("b-block.md#absent", "missing anchor")]:
            text = f"[x][ref]\n\n[ref]: <{target}>\n"
            with self.assertRaisesRegex(AssertionError, error):
                pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_source_root_relative_links_are_rejected(self):
        for url in ("/missing.html", "%2Fmissing.html"):
            for text in (f"[x]({url})", f"[x][ref]\n\n[ref]: {url}\n"):
                with self.assertRaisesRegex(AssertionError, "root-relative link"):
                    pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_links_in_code_are_not_checked(self):
        text = "```markdown\n[example](missing.md)\n```\n\nInline `[x](gone.md)` code.\n"
        pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_explicit_heading_ids_are_anchors(self):
        (self.src / "concepts/parts/b-block.md").write_text("# Block parts\n\n## Landing {#custom}\n")
        pages.validate("[x](b-block.md#custom)", "concepts/parts/a-item.md", self.src, {})
        with self.assertRaisesRegex(AssertionError, "missing anchor"):
            pages.validate("[x](b-block.md#landing-custom)", "concepts/parts/a-item.md", self.src, {})

    def test_missing_compiled_include_anchor_fails(self):
        (self.src / "code.rs").write_text("// ANCHOR: present\n// ANCHOR_END: present\n")
        text = "{{#include ../../code.rs:missing}}"
        with self.assertRaisesRegex(AssertionError, "missing include anchor"):
            pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_rendered_links_must_stay_in_the_book(self):
        output = self.src / "html"
        path = output / "concepts/parts.html"
        path.parent.mkdir(parents=True)
        for href, error in [("/missing.html", "root-relative link"),
                            ("%2Fmissing.html", "root-relative link"),
                            ("missing.html", "missing rendered link"),
                            ("../../../outside.html", "missing rendered link"),
                            ("#absent", "missing rendered anchor")]:
            path.write_text(f'<a href="{href}">x</a>')
            with self.assertRaisesRegex(AssertionError, error):
                pages.check_html(output, self.src / "concepts/..")


if __name__ == "__main__":
    unittest.main()
