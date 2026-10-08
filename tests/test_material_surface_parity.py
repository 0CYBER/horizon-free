import unittest
from tools import test_material_surface_parity as material


class MaterialSurfaceCorpusTests(unittest.TestCase):
    def test_seeded_band_stream_modes_and_boundary_overrides(self):
        fixtures=list(material.corpus())
        self.assertEqual(fixtures,list(material.corpus()))
        real=[f for f in fixtures if f["overrides"]==[None]*3]
        self.assertEqual(len(real),128)
        self.assertEqual({(f["legacy"],f["child"]) for f in real},{(False,False),(False,True),(True,False),(True,True)})
        self.assertEqual({f["consume"] for f in real},{-1,0,1,17})
        values={f["overrides"][0] for f in fixtures if f["overrides"][0] is not None}
        self.assertTrue({"3dffffff","3e000000","3e000001","be000000","7f800000","ff800000","7fc00001"}<=values)
        points=fixtures[0]["points"]
        self.assertIn([7,-193,-11],points)
        self.assertIn([7,-192,-11],points)
        self.assertIn([-2**31,-2**31,2**31-1],points)


if __name__=="__main__":unittest.main()
