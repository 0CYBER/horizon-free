import unittest
from tools import test_material_extension_parity as extension


class MaterialExtensionCorpusTests(unittest.TestCase):
    def test_column_variants_constants_and_seeded_modes(self):
        biomes={'minecraft:plains':'unused','minecraft:frozen_ocean':'unused'}
        fixtures=list(extension.corpus(biomes))
        self.assertEqual(fixtures,list(extension.corpus(biomes)))
        self.assertEqual({f['legacy'] for f in fixtures},{False,True})
        self.assertTrue(any('noise_bits' not in f for f in fixtures))
        self.assertTrue(any('7fc00000' in f.get('noise_bits',[]) for f in fixtures))
        self.assertTrue(any('7f800000' in f.get('noise_bits',[]) for f in fixtures))
        self.assertTrue({1,2,5,9,10,12}<={v for f in fixtures for v in f['initial']})
        self.assertEqual({op[0] for f in fixtures for op in f['ops']},{'badlands','frozen'})
        self.assertTrue(any(f['limit']==32 for f in fixtures))


if __name__=='__main__':unittest.main()
