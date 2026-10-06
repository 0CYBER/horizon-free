import unittest
from tools import test_biome_source_parity as source


class BiomeSourceCorpusTests(unittest.TestCase):
    def test_shift_threshold_failure_and_wrapped_coordinate_coverage(self):
        fixtures = list(source.corpus())
        self.assertEqual(fixtures, list(source.corpus()))
        self.assertEqual({f['kind'] for f in fixtures}, {'fixed', 'checkerboard', 'end'})
        checker = [f for f in fixtures if f['kind'] == 'checkerboard']
        self.assertTrue(set(range(63)) <= {f['scale'] for f in checker})
        self.assertTrue(any(not f['biomes'] for f in checker))
        self.assertTrue(any(len(f['biomes']) != len(set(f['biomes'])) for f in checker))
        self.assertTrue(any(len(f['biomes']) == 512 for f in checker))
        end = [f for f in fixtures if f['kind'] == 'end']
        self.assertTrue(any(f['fail'] for f in end))
        self.assertTrue({'3e800000', 'bd800000', 'be600000', '7fc00000'} <= {f['bits'] for f in end})
        self.assertIn([-(2**31), 0, 2**31-1], fixtures[0]['positions'])
        self.assertIn([256, 0, 0], fixtures[0]['positions'])


if __name__ == '__main__':
    unittest.main()
