"""The SDK reference's tables come from what is published, and a name with no words fails the book's build."""
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.append(str(ROOT / "tools"))

import modding_sdk_tables as tables  # noqa: E402
from tools.ci import nightly  # noqa: E402


class SdkTablesTests(unittest.TestCase):
    def test_every_published_asset_is_in_the_assets_table(self):
        text = tables.assets(nightly.SDK_ASSETS)
        for name in nightly.SDK_ASSETS:
            self.assertIn(f"`{name}`", text)

    def test_an_asset_of_an_unknown_kind_fails(self):
        with self.assertRaises(ValueError):
            tables.assets(nightly.SDK_ASSETS + ("debug-symbols.tar.xz",))

    def test_the_hosts_say_which_have_a_server(self):
        legs = (("x86_64-unknown-linux-gnu", ""), ("x86_64-unknown-linux-gnu", "-server"),
                ("x86_64-pc-windows-gnu", ""), ("aarch64-apple-darwin", ""))
        text = tables.hosts(legs)
        self.assertIn("| Linux, x86-64 | `x86_64-unknown-linux-gnu` | yes | yes |", text)
        self.assertIn("| macOS, Apple silicon | `aarch64-apple-darwin` | yes | no |", text)
        with self.assertRaises(ValueError):
            tables.hosts(legs + (("riscv64gc-unknown-linux-gnu", ""),))

    def test_the_routes_come_from_the_input_list(self):
        text = tables.routes(json.loads(tables.INPUTS.read_text()))
        self.assertIn("`runtime`, `api`, `engine`", text)

    def test_surface_records_are_counted_by_kind_with_members_apart(self):
        others = [{"id": record} for record in ("trait:Host", "method:Host::level", "assoc:Host::Side",
                                                "handle:Pos::new", "fn:log", "macro:hook", "part:items",
                                                "capability:net", "behaviour:pool_merging")]
        document = {"schema": 2, "hooks": [{"id": "hook:tick", "fields": [{"id": "hook:tick.phase"}]}],
                    "types": [{"id": "type:world::Pos", "fields": [{"id": "type:world::Pos.x"},
                                                                   {"id": "type:world::Pos.y"}]}],
                    "others": others}
        text = tables.surface(document)
        self.assertIn("| Public field or variant of a type | `type:<path>.<field or variant>` | 2 |", text)
        self.assertIn("| Hook: a moment the game tells mods about | `hook:<name>` | 1 |", text)
        with self.assertRaises(ValueError):
            tables.surface({**document, "hooks": [{"id": "event:tick"}]})

    def test_words_for_a_surface_record_kind_that_is_gone_fail(self):
        with self.assertRaises(ValueError) as raised:
            tables.surface({"schema": 2, "hooks": [{"id": "hook:tick"}]})
        self.assertIn("'fn'", str(raised.exception))

    def test_a_marker_no_chapter_holds_fails(self):
        book = {"sections": [{"Chapter": {"content": "{{#sdk-assets}} {{#sdk-hosts}} {{#sdk-routes}}",
                                          "sub_items": []}}]}
        with self.assertRaises(ValueError) as raised:
            tables.preprocess({"root": str(ROOT / "docs/modding")}, book)
        self.assertIn("{{#sdk-surface}}", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
