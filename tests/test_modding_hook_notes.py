"""Generated chapter navigation and links must work without a source Markdown page."""
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

with patch.object(sys, "path", [*sys.path, str(Path(__file__).resolve().parents[1] / "tools")]):
    import modding_hook_notes as notes


class HookNotes(unittest.TestCase):
    def book(self):
        return {"sections": [{"Chapter": dict(name="Reference", path="reference/api.md",
            content="", number=[1], parent_names=[], sub_items=[{"Chapter": dict(
                name="Hook notes", path=None, source_path=None, content="", number=[1, 1],
                parent_names=["Reference"], sub_items=[])}])}]}

    def test_virtual_chapter_keeps_its_url_navigation_and_generated_content(self):
        book = notes.expand(self.book(), "# Hook notes\n\n## Example\n\nFresh notes.", "Hook notes", notes.PAGE)
        chapter = book["sections"][0]["Chapter"]["sub_items"][0]["Chapter"]
        self.assertEqual(chapter["path"], "reference/hook-notes.md")
        self.assertIsNone(chapter["source_path"])
        self.assertEqual(chapter["number"], [1, 1])
        self.assertEqual(chapter["parent_names"], ["Reference"])
        self.assertIn("Fresh notes.", chapter["content"])

    def test_missing_or_duplicate_draft_chapter_fails(self):
        for book in ({"sections": []}, self.book()):
            if book["sections"]:
                book["sections"].extend(self.book()["sections"])
            with self.assertRaisesRegex(AssertionError, "one draft"):
                notes.expand(book, "generated", "Hook notes", notes.PAGE)

    def test_catalog_failure_is_not_silently_replaced_with_an_empty_page(self):
        with patch.object(notes.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "cargo")):
            with self.assertRaises(subprocess.CalledProcessError):
                notes.preprocess({"root": "/checkout/docs/modding"}, self.book())

    def test_uses_mdbook_root_not_callers_working_directory(self):
        with patch.object(notes.subprocess, "run", return_value=subprocess.CompletedProcess(
                [], 0, stdout="# Hook notes\n\nLatest.")) as run:
            result = notes.preprocess({"root": "/checkout/docs/modding"}, self.book())
        self.assertEqual(run.call_args.kwargs["cwd"], Path("/checkout").resolve())
        self.assertTrue(run.call_args.kwargs["check"])
        self.assertIn("Latest.", result["sections"][0]["Chapter"]["sub_items"][0]["Chapter"]["content"])



if __name__ == "__main__":
    unittest.main()
