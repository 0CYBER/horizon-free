"""Pure-logic checks for the helpers behind lan_e2e's `mobs` check (no client, no server)."""
import importlib.util
import sys
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent.parent / 'tools'
# Loaded by path: putting tools/ on sys.path would shadow same-named test modules in tests/ during discovery.
_spec = importlib.util.spec_from_file_location('lan_e2e', TOOLS / 'lan_e2e.py')
lan_e2e = importlib.util.module_from_spec(_spec)
sys.modules['lan_e2e'] = lan_e2e
# lan_e2e puts tools/ first on sys.path when imported; restore it so discovery still finds tests/ modules.
_saved_path = list(sys.path)
try:
    _spec.loader.exec_module(lan_e2e)
finally:
    sys.path[:] = _saved_path


class NearestMobDistance(unittest.TestCase):
    NEAR = [
        {'kind': 'minecraft:zombie', 'pos': [10.0, 64.0, 13.0], 'health': 20.0},
        {'kind': 'minecraft:zombie', 'pos': [10.0, 64.0, 11.5], 'health': 0.0},
        {'kind': 'minecraft:skeleton', 'pos': [10.0, 64.0, 10.5], 'health': 20.0},
    ]

    def test_closest_living_zombie(self):
        # Only the living zombie counts: the dead one (health 0) is 1.5 blocks away and is ignored.
        self.assertEqual(lan_e2e.nearest_mob_distance(self.NEAR, 'minecraft:zombie', (10.0, 64.0, 10.0)), 3.0)

    def test_other_kind_is_ignored(self):
        self.assertEqual(lan_e2e.nearest_mob_distance(self.NEAR, 'minecraft:skeleton', (10.0, 64.0, 10.0)), 0.5)

    def test_none_when_absent(self):
        self.assertIsNone(lan_e2e.nearest_mob_distance(self.NEAR, 'minecraft:cow', (10.0, 64.0, 10.0)))
        self.assertIsNone(lan_e2e.nearest_mob_distance([], 'minecraft:zombie', (0.0, 0.0, 0.0)))


if __name__ == '__main__':
    unittest.main()
