import unittest
from tools import test_piston_shape_parity as piston


class PistonShapeCorpusTests(unittest.TestCase):
    def test_source_extension_thresholds_noclip_and_float_interpolation(self):
        facts={'states':[{'id':name} for name in ['minecraft:stone','minecraft:piston','minecraft:shulker_box','minecraft:moving_piston']], 'defaults':{'minecraft:stone':0}}
        fixtures=list(piston.corpus(facts))
        self.assertEqual(fixtures,list(piston.corpus(facts)))
        self.assertEqual({f['noclip'] for f in fixtures},set(range(-1,6)))
        self.assertEqual({f['direction'] for f in fixtures},set(range(6)))
        self.assertEqual({(f['source'],f['extending']) for f in fixtures},{(s,e) for s in [False,True] for e in [False,True]})
        self.assertEqual(sum(f['kinematic'] for f in fixtures),10000)
        self.assertTrue(any(f['fail']==0 for f in fixtures))
        for field in ['progress','previous','partial']:
            self.assertTrue(all(len(f[field])==8 for f in fixtures))


if __name__=='__main__':unittest.main()
