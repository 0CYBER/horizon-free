import unittest
from tools import test_feature_region_parity as regions


class FeatureRegionCorpusTests(unittest.TestCase):
    def test_resident_mutation_contexts_and_failures(self):
        fixtures = list(regions.corpus({}))
        self.assertEqual(fixtures, list(regions.corpus({})))
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
