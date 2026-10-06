import json
import unittest
from tools import test_scattered_ore_parity as ore


class ScatteredOreCorpusTests(unittest.TestCase):
    def test_exposure_directions_empty_targets_and_boundary_sizes(self):
        fixtures = list(ore.corpus({}))
        self.assertEqual({f['mode'] for f in fixtures}, set(range(8)))
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 7})
        definitions = [json.loads(f['definition']) for f in fixtures]
        sizes = {d['size'] for d in definitions}
        self.assertTrue({-1, 0, 1, 7, 8, 64, 65, 4294967297} <= sizes)
        self.assertTrue(any(d['targets'] == [] for d in definitions))
        self.assertTrue(any(f['origin'][0] == 2147483647 for f in fixtures))
        self.assertTrue(any(f['origin'][0] == -2147483648 for f in fixtures))


if __name__ == '__main__':
    unittest.main()
