"""`split_part_table.top_items` reads a parts file's top-level items, and skips what a parts file cannot export."""
import unittest

from tools.hotspots.split_part_table import top_items


class TopItemsTest(unittest.TestCase):
    def test_extern_crate_is_skipped_and_an_extern_abi_fn_is_kept(self):
        source = 'pub extern crate foo;\nextern "C" fn c_abi() {}\nfn plain() {}\n'
        items = top_items(source)
        self.assertEqual([(item["kind"], item["name"]) for item in items],
                         [("fn", "c_abi"), ("fn", "plain")])
