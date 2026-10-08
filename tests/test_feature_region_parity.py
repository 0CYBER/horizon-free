import unittest
from tools import test_feature_region_parity as regions


class FeatureRegionCorpusTests(unittest.TestCase):
    def test_resident_mutation_contexts_and_failures(self):
        facts = {'defaults': {'minecraft:water': 0, 'minecraft:oak_log': 1},
                 'states': [[None, None, None, None, {'id': 'minecraft:water'}],
                            [None, None, None, None, {'id': 'minecraft:oak_log', 'properties': {'axis': 'y'}}],
                            [None, None, None, None, {'id': 'minecraft:oak_log', 'properties': {'axis': 'x'}}]]}
        fixtures = list(regions.corpus(facts))
        self.assertEqual(fixtures, list(regions.corpus(facts)))
        self.assertEqual(sorted(i for f in fixtures for i in f['probe_states']), list(range(len(facts['states']))))
        self.assertTrue(all(len(f['tick_commands']) == 77 for f in fixtures))
        self.assertTrue(all(any(v['type'] == 'spring_feature' for v in f['features']) for f in fixtures))
        self.assertTrue(all([v['active'] for v in f['features'] if v['type'] == 'end_podium'] == [False, True] for f in fixtures))
        self.assertEqual({None if f['write_zone'] is None else tuple(f['write_zone']) for f in fixtures}, {None, (0, 0, 0), (1, 1, 0), (-1, -1, 1), (0, 0, -1)})
        self.assertEqual({f['upgrading'] for f in fixtures}, {False, True})
        self.assertEqual({f['worldgen'] for f in fixtures}, {False, True})
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['stage'] for f in fixtures}, {'empty', 'terrain'})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 7})
        self.assertEqual({f['reject'] for f in fixtures}, {0, 3})
        self.assertTrue(all(f['features'][3]['type'] == 'sequence' for f in fixtures))
        self.assertEqual(len(fixtures[0]['features'][3]['features']), 4)
        kinds = {m['type'] for f in fixtures for m in f['placement']}
        self.assertTrue({'heightmap', 'count_on_every_layer', 'in_square'} <= kinds)


if __name__ == '__main__':
    unittest.main()
