import math
import unittest
from tools import test_face_support_parity as faces


class FaceSupportCorpusTests(unittest.TestCase):
    def test_sparse_grids_face_boundaries_and_translations(self):
        fixtures=list(faces.corpus())
        self.assertEqual(fixtures,list(faces.corpus()))
        self.assertTrue(any(f.get('boxes')==[] for f in fixtures))
        self.assertTrue(any('move' in f for f in fixtures))
        self.assertTrue(any(f.get('cube') for f in fixtures))
        self.assertEqual(len(fixtures[:512]),512)
        for f in fixtures:
            if 'coords' in f:self.assertEqual(len(f['occupied']),math.prod(len(v)-1 for v in f['coords']))
            else:
                for box in f['boxes']:self.assertEqual(len(box),6)


if __name__=='__main__':unittest.main()
