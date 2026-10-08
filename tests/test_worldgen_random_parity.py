import unittest
from tools import test_worldgen_random_parity as random


class WorldgenRandomCorpusTests(unittest.TestCase):
    def test_wrapper_layers_overflow_seeds_and_fork_operations(self):
        fixtures = list(random.corpus())
        self.assertEqual(fixtures, list(random.corpus()))
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['layers'] for f in fixtures}, {1, 2, 3})
        self.assertEqual({f['warmup'] for f in fixtures}, {0, 7})
        operations = {op[0] for f in fixtures for op in f['ops']}
        self.assertTrue({'decoration', 'feature', 'large', 'salt', 'slime', 'fork', 'pos', 'pos_seed', 'pos_hash', 'bits'} <= operations)
        bits = {op[1] for f in fixtures for op in f['ops'] if op[0] == 'bits'}
        self.assertTrue({-2147483648, 0, 32, 64, 2147483647} <= bits)
        self.assertTrue(any(op[0] == 'feature' and op[3] == 214749 for f in fixtures for op in f['ops']))


if __name__ == '__main__':
    unittest.main()
