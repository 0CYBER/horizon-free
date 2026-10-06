import unittest
from tools import test_carving_geometry_parity as geometry


class CarvingGeometryCorpusTests(unittest.TestCase):
    def test_shape_boundaries_ieee_values_and_output_failures(self):
        fixtures = list(geometry.corpus())
        self.assertEqual(fixtures, list(geometry.corpus()))
        self.assertEqual(len(fixtures), 15)
        shapes = [e for f in fixtures for e in f['ellipsoids']]
        self.assertEqual(len(shapes), 690)
        self.assertEqual({e['skip'] for e in shapes}, {0, 1, 2, 3})
        self.assertEqual({e['fail'] for e in shapes}, {-1, 0, 1, 63})
        self.assertIn(geometry.bits(float('nan')), {e['horizontal'] for e in shapes})
        self.assertIn(geometry.bits(-0.), {e['vertical'] for e in shapes})
        reach = list(geometry.reach_corpus())
        self.assertEqual(reach, list(geometry.reach_corpus()))
        self.assertTrue(any(q[4] == -2**31 and q[5] == 2**31 - 1 for q in reach))


if __name__ == '__main__':
    unittest.main()
