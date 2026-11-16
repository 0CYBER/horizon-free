"""The reference is a live, virtual chapter, not a committed catalog snapshot."""
from contextlib import chdir
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tomllib
import tempfile
import unittest
from unittest.mock import patch

with patch.object(sys, "path", [*sys.path, str(Path(__file__).resolve().parents[1] / "tools")]):
    import neoforge_counterparts as counterparts


class Counterparts(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.hooks = self.root / "crates/horizon-mod-vocab/src/hooks/example.rs"
        self.hooks.parent.mkdir(parents=True)
        self.context = {"root": str(self.root / "docs/modding")}

    def book(self):
        return {"sections": [{"Chapter": dict(name="Reference", path="reference/api.md",
            content="", number=[1], parent_names=[], sub_items=[{"Chapter": dict(
                name="NeoForge counterparts", path=None, source_path=None, content="", number=[1, 2],
                parent_names=["Reference"], sub_items=[])}])}]}

    def chapter(self):
        result = counterparts.preprocess(self.context, self.book())
        return result["sections"][0]["Chapter"]["sub_items"][0]["Chapter"]

    def test_virtual_reference_uses_current_declarations_and_keeps_published_url(self):
        self.hooks.write_text('#[hook(side = "client", neoforge = "IItemDecorator.render")] fn decoration();')
        chapter = self.chapter()
        self.assertEqual(chapter["path"], "reference/neoforge-counterparts.md")
        self.assertIsNone(chapter["source_path"])
        self.assertEqual(chapter["number"], [1, 2])
        self.assertEqual(chapter["parent_names"], ["Reference"])
        self.assertIn("| `decoration` | client | `IItemDecorator.render` |", chapter["content"])
        self.hooks.write_text('#[hook(side = "server", neoforge = "IPayloadHandler.handle")] fn received();')
        content = self.chapter()["content"]
        self.assertIn("| `received` | server | `IPayloadHandler.handle` |", content)
        self.assertNotIn("`decoration`", content)
        self.assertFalse((self.root / "docs/modding/src/reference/neoforge-counterparts.md").exists())

    def test_notes_between_the_hook_and_its_fn_keep_the_row(self):
        self.hooks.write_text('''
#[hook(side = "client", contract = "listen", neoforge = "GuiLayer.render")]
#[notes = r#"A note may hold `)]`, `fn fake()` and a raw string."#]
fn draw();
''')
        content = self.chapter()["content"]
        self.assertIn("| `draw` | client | `GuiLayer.render` |", content)
        self.assertIn("1 hooks: 1 name a counterpart, 0 say they have none.", content)

    def test_comments_inside_attributes_are_skipped(self):
        self.hooks.write_text('''
#[hook(
    side = "client", // a comment with ] and neoforge = "FakeEvent"
    /* a block comment with ] and fn fake() /* nested */ still a comment */
    contract = "listen",
    neoforge = "GuiLayer.render"
)]
fn draw();
''')
        content = self.chapter()["content"]
        self.assertIn("| `draw` | client | `GuiLayer.render` |", content)
        self.assertNotIn("FakeEvent", content)

    def test_hooks_inside_comments_and_notes_are_not_rows(self):
        self.hooks.write_text('''
// #[hook(side = "server", neoforge = "FakeEvent")] fn fake();
#[hook(side = "client", neoforge = "GuiLayer.render")]
#[notes = r#"An example: #[hook(side = "server", neoforge = "FakeEvent")] fn fake();"#]
fn draw();
''')
        content = self.chapter()["content"]
        self.assertIn("| `draw` | client | `GuiLayer.render` |", content)
        self.assertNotIn("FakeEvent", content)
        self.assertNotIn("`fake`", content)
        self.assertIn("1 hooks: 1 name a counterpart, 0 say they have none.", content)

    def test_context_root_is_independent_of_the_callers_working_directory(self):
        self.hooks.write_text('''
#[hook(side = "both", neoforge = ["FirstEvent", "SecondEvent"])] fn first();
#[hook(side = "client", neoforge = "none (runtime callback, not registration)")] fn second();
''')
        with tempfile.TemporaryDirectory() as elsewhere, chdir(elsewhere):
            content = self.chapter()["content"]
        self.assertIn("2 hooks: 1 name a counterpart, 1 say they have none.", content)
        self.assertIn("| `first` | both | `FirstEvent, SecondEvent` |", content)
        self.assertIn("| `second` | client | runtime callback, not registration |", content)

    def test_book_command_accepts_the_mdbook_protocol_outside_the_checkout(self):
        root = Path(__file__).resolve().parents[1]
        config = tomllib.loads((root / "docs/modding/book.toml").read_text())
        command = shlex.split(config["preprocessor"]["neoforge-counterparts"]["command"])
        context = {"root": str(root / "docs/modding")}
        with tempfile.TemporaryDirectory() as elsewhere:
            supported = subprocess.run([*command, "supports", "html"], cwd=elsewhere,
                check=True, capture_output=True, text=True)
            self.assertEqual(supported.stdout, "")
            result = subprocess.run(command, cwd=elsewhere, check=True, capture_output=True,
                text=True, input=json.dumps([context, self.book()]))
        book = json.loads(result.stdout)
        chapter = book["sections"][0]["Chapter"]["sub_items"][0]["Chapter"]
        self.assertIn("| `item_decorations` | client | `IItemDecorator.render` |", chapter["content"])

    def test_missing_draft_chapter_fails_instead_of_dropping_the_reference(self):
        self.hooks.write_text("")
        with self.assertRaisesRegex(AssertionError, "one draft"):
            counterparts.preprocess(self.context, {"sections": []})


if __name__ == "__main__":
    unittest.main()
