import json
import unittest
from tools import test_fill_layer_parity as fill


class FillLayerCorpusTests(unittest.TestCase):
    def test_exhaustive_states_and_column_failure_cases(self):
        states = ['stone', {'id': 'oak_log', 'properties': {'axis': 'x'}}]
        fixtures = list(fill.corpus({'states': states}))
        decode = [f for f in fixtures if f['repeats'] == 0]
        self.assertEqual([json.loads(f['definition'])['state'] for f in decode], states)
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 1, 7, 63})
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['world_min'] for f in fixtures}, {-64, -2147483648, 2147483647})
        heights = {json.loads(f['definition'])['height'] for f in fixtures}
        self.assertTrue({-1, 0, 4064, 4065, 4294967297}.issubset(heights))


if __name__ == '__main__':
    unittest.main()
