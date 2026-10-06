import json
import unittest
from tools import test_placement_parity as placement


class PlacementCorpusTests(unittest.TestCase):
    def test_modifier_kinds_callback_failures_and_overflow(self):
        definitions = list(placement.definitions())
        self.assertEqual(definitions, list(placement.definitions()))
        values = [json.loads(raw) for raw in definitions]
        kinds = {v.get('type') for v in values if isinstance(v, dict)}
        self.assertTrue({'block_predicate_filter', 'environment_scan', 'count_on_every_layer', 'cuboid', 'noise_based_count', 'noise_threshold_count', 'heightmap', 'surface_relative_threshold_filter', 'surface_water_depth_filter', 'count', 'in_square', 'offset', 'height_range', 'rarity_filter', 'random_chance', 'fixed_placement', 'randomly_selected'} <= kinds)
        fixtures = list(placement.corpus())
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 2})
        self.assertEqual({f['draw'] for f in fixtures}, {False, True})
        self.assertEqual({f['height_mode'] for f in fixtures}, {0, 1})
        self.assertEqual({f['count_only'] for f in fixtures}, {False, True})
        self.assertTrue(any(f['query_fail'] == 1 for f in fixtures))
        self.assertTrue(any(f['world_min'] != f['context'][0] for f in fixtures))
        self.assertTrue(any(f['context'][0] == 2**31 - 1 for f in fixtures))
        self.assertTrue(any('4294967297' in f['definition'] for f in fixtures))
        self.assertTrue(all(len(f['positions']) == 4 for f in fixtures))


if __name__ == '__main__':
    unittest.main()
