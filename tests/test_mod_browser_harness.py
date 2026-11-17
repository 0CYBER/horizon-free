"""Pure-Python regression tests for tools/mod_browser_harness.py (no game or launcher needed)."""
import json
import tempfile
import unittest
from pathlib import Path

from tools import mod_browser_harness as mb


def write_recipe(mod, body):
    folder = mod / "data" / "modns" / "recipe"
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "thing.json").write_text(json.dumps(body))


class PickRecipeTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.mod = Path(tmp.name)

    def shapeless(self, ingredients):
        write_recipe(self.mod, {
            "type": "minecraft:crafting_shapeless",
            "ingredients": ingredients,
            "result": {"id": "modns:thing", "count": 1},
        })
        return mb.pick_recipe(self.mod, "modns")

    def test_object_form_ingredients_are_picked_and_normalised(self):
        # 1.20.5+ serialises an ingredient as {"item": "minecraft:id"}, not just a bare id.
        recipe = self.shapeless([{"item": "minecraft:stick"}, {"item": "minecraft:stick"}])
        self.assertEqual(recipe["result"]["id"], "modns:thing")
        self.assertEqual(mb.grid_cells(recipe), [(100, "minecraft:stick"), (101, "minecraft:stick")])

    def test_bare_string_ingredients_still_work(self):
        recipe = self.shapeless(["minecraft:stick", "minecraft:stick"])
        self.assertEqual(mb.grid_cells(recipe), [(100, "minecraft:stick"), (101, "minecraft:stick")])

    def test_shaped_object_form_ingredients_are_picked_and_normalised(self):
        write_recipe(self.mod, {
            "type": "minecraft:crafting_shaped",
            "pattern": ["SS"],
            "key": {"S": {"item": "minecraft:stick"}},
            "result": {"id": "modns:thing", "count": 1},
        })
        recipe = mb.pick_recipe(self.mod, "modns")
        self.assertEqual(mb.grid_cells(recipe), [(100, "minecraft:stick"), (101, "minecraft:stick")])

    def test_non_vanilla_object_form_ingredient_is_rejected(self):
        with self.assertRaises(mb.Failed):
            self.shapeless([{"item": "otherns:thing"}])


if __name__ == "__main__":
    unittest.main()
