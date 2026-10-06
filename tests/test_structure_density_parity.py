import unittest
from tools import test_structure_density_parity as structure


class StructureDensityCorpusTests(unittest.TestCase):
    def test_modes_kernel_neighborhood_and_null_bounds_are_covered(self):
        fixtures=list(structure.corpus())
        self.assertEqual(fixtures,list(structure.corpus()))
        self.assertEqual({p[1] for f in fixtures for p in f["pieces"]},{"NONE","BURY","BEARD_THIN","BEARD_BOX","ENCAPSULATE"})
        kernel=fixtures[0]
        self.assertEqual(kernel["junctions"],[[0,0,0]])
        self.assertIn([29,29,29,-14,-14,-14,1,1,1,3],kernel["volumes"])
        self.assertTrue(any(f["affected"] is None and f["pieces"] for f in fixtures))
        self.assertTrue(any(p[2]==-2**31 for f in fixtures for p in f["pieces"]))
        self.assertTrue(any(p[2]==2**31-1 for f in fixtures for p in f["pieces"]))

    def test_clipped_stride_fixture_exercises_a_sample_below_bounds(self):
        fixture=next(f for f in structure.corpus() if f["affected"]==[1,1,1,9,9,9])
        volume=fixture["volumes"][0]
        for axis in range(3):
            minimum=volume[axis+3];step=volume[axis+6];bound=fixture["affected"][axis]
            first=minimum+max(0,bound-minimum)//step*step
            self.assertLess(first,bound)
        self.assertGreater(volume[9],0)


if __name__=="__main__":unittest.main()
