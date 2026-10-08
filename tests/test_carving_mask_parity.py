import unittest
from tools import test_carving_mask_parity as carving


class CarvingMaskCorpusTests(unittest.TestCase):
    def test_sparse_dense_wrap_and_mutating_traversal_coverage(self):
        fixtures = list(carving.corpus())
        self.assertEqual(fixtures, list(carving.corpus()))
        self.assertTrue(any(f['max'] - f['min'] + 1 == 4064 for f in fixtures))
        self.assertTrue(any(f['min'] == -2**31 for f in fixtures))
        self.assertTrue(any(f['max'] == 2**31 - 1 for f in fixtures))
        ops = [op for f in fixtures for op in f['ops']]
        self.assertEqual({op['kind'] for op in ops}, {'carve', 'fill', 'visit', 'read'})
        self.assertTrue(any(op.get('fail') == 0 and op.get('add') for op in ops))
        self.assertTrue(any(op.get('fail') == 63 for op in ops))
        self.assertTrue(any(op.get('pos', [0])[0] == -2**31 for op in ops))


if __name__ == '__main__':
    unittest.main()
