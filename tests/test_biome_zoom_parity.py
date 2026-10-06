import unittest
from tools import test_biome_zoom_parity as zoom


class BiomeZoomCorpusTests(unittest.TestCase):
    def test_seed_coordinate_and_double_boundary_coverage(self):
        fixtures = list(zoom.corpus())
        self.assertEqual(fixtures, list(zoom.corpus()))
        self.assertEqual({f[0] for f in fixtures}, {'s', 'b', 'p'})
        self.assertIn('s -9223372036854775808', fixtures)
        self.assertIn('s 9223372036854775807', fixtures)
        self.assertIn('b 0 -2147483648 0 2147483647', fixtures)
        self.assertIn('b 1 -1 0 1', fixtures)
        self.assertIn('p 7ff0000000000000 fff0000000000000 7ff8000000000000', fixtures)
        self.assertIn('p 0000000000000001 8000000000000001 fff8000000000000', fixtures)
        self.assertEqual(sum(f.startswith('s ') for f in fixtures), 4104)


if __name__ == '__main__':
    unittest.main()
