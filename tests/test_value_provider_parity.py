import json
import unittest
from tools import test_value_provider_parity as providers


class ValueProviderCorpusTests(unittest.TestCase):
    def test_distribution_branches_overflow_and_random_modes(self):
        definitions = list(providers.definitions())
        self.assertEqual(definitions, list(providers.definitions()))
        objects = [json.loads(raw) for _, raw in definitions if raw.startswith('{')]
        kinds = {v.get('type') for v in objects if isinstance(v.get('type'), str)}
        self.assertTrue({'weighted_list', 'constant', 'uniform', 'trapezoid', 'clamped', 'biased_to_bottom', 'very_biased_to_bottom'} <= kinds)
        self.assertTrue(any('"value":-0.0' in raw for _, raw in definitions))
        self.assertTrue(any('4294967296' in raw for _, raw in definitions))
        self.assertTrue(any(v.get('plateau') == -2**31 for v in objects))
        fixtures = list(providers.corpus())
        self.assertEqual(len(fixtures), len(definitions) * 10)
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['samples'] for f in fixtures}, {64})


if __name__ == '__main__':
    unittest.main()
