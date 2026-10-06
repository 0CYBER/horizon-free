import json
import unittest
from tools import test_basic_feature_parity as features


class BasicFeatureCorpusTests(unittest.TestCase):
    def test_native_feature_types_partial_mutations_and_wrapped_origins(self):
        fixtures = list(features.corpus())
        self.assertEqual(fixtures, list(features.corpus()))
        kinds = {v.get('type') for f in fixtures if isinstance(v := json.loads(f['definition']), dict)}
        self.assertTrue({'void_start_platform', 'end_island', 'end_platform'} <= kinds)
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 7, 63})
        self.assertEqual({f['reject'] for f in fixtures}, {0, 3})
        self.assertTrue(any(f['origin'][0] == 2**31 - 1 for f in fixtures))
        self.assertTrue(any(f['origin'][1] == -2**31 for f in fixtures))
        self.assertTrue(any(f['repeats'] == 2 for f in fixtures))
        composed = [f for f in fixtures if 'placement' in f]
        self.assertTrue(any(not f['placement'] for f in composed))
        self.assertTrue(any(len(f['placement']) > 1 and f['fail'] == 7 for f in composed))
        self.assertTrue(any(any(p['type'] == 'height_range' for p in f['placement']) for f in composed))


if __name__ == '__main__':
    unittest.main()
