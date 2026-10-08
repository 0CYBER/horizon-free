import unittest
from tools import test_biome_placement_parity as biome


class BiomePlacementCorpusTests(unittest.TestCase):
    def test_context_membership_nested_placers_and_partial_failures(self):
        fixtures = list(biome.corpus())
        self.assertEqual({f['top'] for f in fixtures}, {False, True})
        self.assertEqual({f['direct'] for f in fixtures}, {False, True})
        self.assertEqual({f['nested'] for f in fixtures}, {False, True})
        self.assertEqual({f['worldgen'] for f in fixtures}, {False, True})
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({mode for f in fixtures for mode in f['membership']}, {0, 1, 2, 3})
        self.assertTrue(all(f['placement'] == [{'type': 'biome'}] for f in fixtures if f['direct']))
        self.assertTrue(any(f['query_fail'] == 1 for f in fixtures))
        self.assertTrue(any(f['feature_fail'] == 2 for f in fixtures))


if __name__ == '__main__':
    unittest.main()
