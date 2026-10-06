import unittest
from tools import test_feature_placement_parity as placement


class FeaturePlacementCorpusTests(unittest.TestCase):
    def test_chains_repeated_failure_and_boolean_results(self):
        chains = list(placement.definitions())
        self.assertEqual(chains, list(placement.definitions()))
        self.assertIn([], chains)
        self.assertTrue(any(len(chain) >= 4 for chain in chains))
        fixtures = list(placement.corpus())
        self.assertEqual(len(fixtures), sum(36 if any(v.get("type") == "count_on_every_layer" for v in chain) else 72 for chain in chains))
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 2, 5})
        self.assertEqual({f['result'] for f in fixtures}, {0, 1, 2})
        self.assertTrue(any(f['query_fail'] == 1 for f in fixtures))
        self.assertTrue(all(f['positions'][0] == f['positions'][1] for f in fixtures))


if __name__ == '__main__':
    unittest.main()
