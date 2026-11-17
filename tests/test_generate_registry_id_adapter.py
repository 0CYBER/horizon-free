"""state_substitutes keeps NONE when a substitute block's states have no old counterpart."""
import unittest

from tools.generate_registry_id_adapter import NONE, state_substitutes


class StateSubstitutesEmptyCandidates(unittest.TestCase):
    def test_disjoint_substitute_state_properties_leave_the_pin_unmapped(self):
        # pink_wool_slab matches the wool/concrete-slab substitute rule -> smooth_stone_slab.
        # The substitute exists in both versions but with disjoint state properties, so no new
        # substitute state also exists in the old version to carry the mapping. max() on that
        # empty candidate list used to abort generation with ValueError.
        new_blocks = {
            "minecraft:smooth_stone_slab": {"states": [{"id": 0, "properties": {"a": "1"}}]},
            "minecraft:pink_wool_slab": {"states": [{"id": 1, "properties": {}}]},
        }
        old_blocks = {
            "minecraft:smooth_stone_slab": {"states": [{"id": 0, "properties": {"b": "1"}}]},
        }
        self.assertEqual(state_substitutes(old_blocks, new_blocks), [NONE, NONE])

    def test_a_shared_substitute_state_still_maps_the_pin(self):
        # Control: the empty-candidate guard must not disable a valid mapping.
        new_blocks = {
            "minecraft:smooth_stone_slab": {"states": [{"id": 0, "properties": {"a": "1"}}]},
            "minecraft:pink_wool_slab": {"states": [{"id": 1, "properties": {}}]},
        }
        old_blocks = {
            "minecraft:smooth_stone_slab": {"states": [{"id": 0, "properties": {"a": "1"}}]},
        }
        self.assertEqual(state_substitutes(old_blocks, new_blocks), [NONE, 0])


if __name__ == "__main__":
    unittest.main()
