import json
import unittest
from tools import test_height_provider_parity as height


class HeightProviderCorpusTests(unittest.TestCase):
    def test_anchor_variants_inner_plateaus_and_overflow_contexts(self):
        definitions = list(height.definitions())
        self.assertEqual(definitions, list(height.definitions()))
        values = [json.loads(raw) for raw in definitions]
        objects = [v for v in values if isinstance(v, dict)]
        self.assertTrue({'weighted_list', 'constant', 'uniform', 'biased_to_bottom', 'very_biased_to_bottom', 'trapezoid'} <= {v.get('type') for v in objects})
        self.assertTrue(any(v.get('absolute') == 5 and v.get('type') == 'uniform' for v in objects))
        self.assertTrue(any(v.get('inner') == 0 for v in objects))
        self.assertTrue(any(v.get('plateau') == -2**31 for v in objects))
        fixtures = list(height.corpus())
        self.assertEqual(len(fixtures), len(definitions) * 30)
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertTrue(any(f['context'][0] == -2**31 for f in fixtures))


if __name__ == '__main__':
    unittest.main()
