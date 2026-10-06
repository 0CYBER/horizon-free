import unittest
from tools import test_block_properties_parity as properties


class BlockPropertiesCorpusTests(unittest.TestCase):
    def test_cache_flags_empty_shapes_and_threshold_cases(self):
        fixtures=list(properties.corpus())
        self.assertEqual(fixtures,list(properties.corpus()))
        self.assertEqual({(f['on'],f['off'],f['cached']) for f in fixtures},
                         {(a,b,c) for a in [False,True] for b in [False,True] for c in [False,True]})
        self.assertEqual({f['occludes'] for f in fixtures},{False,True})
        self.assertTrue(any(not f['shape'] for f in fixtures))
        self.assertTrue(any(len(f['shape'])>1 for f in fixtures))
        self.assertGreater(len(fixtures),8000)
        for f in fixtures:
            for box in f['shape']:self.assertEqual(len(box),6)


if __name__=='__main__':unittest.main()
