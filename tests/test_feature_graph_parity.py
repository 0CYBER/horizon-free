import json
import unittest
from tools import test_feature_graph_parity as graphs


class FeatureGraphCorpusTests(unittest.TestCase):
    def test_composition_decoding_failure_and_shared_randomness(self):
        definitions = list(graphs.definitions())
        self.assertEqual(definitions, list(graphs.definitions()))
        self.assertEqual({d['type'] for d in definitions}, {'no_op', 'overlay', 'sequence', 'random_boolean_selector', 'simple_random_selector', 'random_selector', 'weighted_random_selector'})
        self.assertTrue(any(d.get('features') == [] for d in definitions))
        fixtures = list(graphs.corpus())
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 7})
        self.assertEqual({f['reject'] for f in fixtures}, {0, 3})
        self.assertTrue(any('placement' in f for f in fixtures))
        self.assertTrue(any('-0.0' in f['definition'] for f in fixtures))
        self.assertTrue(all(isinstance(json.loads(f['definition']), dict) for f in fixtures))


if __name__ == '__main__':
    unittest.main()
