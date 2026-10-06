import unittest
import json
from tools import test_biome_temperature_parity as biome


class BiomeTemperatureCorpusTests(unittest.TestCase):
    def test_temperature_edges_packed_aliases_and_cache_eviction(self):
        fixtures=list(biome.corpus())
        self.assertEqual(fixtures,list(biome.corpus()))
        self.assertEqual(len(fixtures),204)
        self.assertEqual({(f['modifier'],f['precipitation']) for f in fixtures},{('NONE',False),('NONE',True),('FROZEN',False),('FROZEN',True)})
        self.assertTrue({'3e199999','3e19999a','3e19999b','7f800000','ff800000','7fc00000'}<={f['base'] for f in fixtures})
        isolated=[f for f in fixtures if any(op[0]=='thread' for op in f['ops'])]
        self.assertEqual(len(isolated),12)
        self.assertEqual({f['ops'][2][1] for f in isolated},{0,1023})
        self.assertTrue(all(f['ops'][0]==['fill',1024,0,63] for f in isolated))
        queries=[op[1:] for op in fixtures[0]['ops'] if op[0]=='query']
        self.assertIn([0,80,0,63],queries)
        self.assertIn([0,81,0,63],queries)
        self.assertIn([2**26,4032,-2**26,63],queries)

    def test_climate_codec_fields_and_float_rounding_edges(self):
        fixtures=list(biome.codec_corpus({'horizon:test':'{"has_precipitation":false,"temperature":0,"downfall":1}'}))
        self.assertEqual(fixtures,list(biome.codec_corpus({'horizon:test':'{"has_precipitation":false,"temperature":0,"downfall":1}'})))
        decoded=[json.loads(f['json']) for f in fixtures]
        self.assertIn({'has_precipitation':True,'temperature':0.5,'downfall':0.5},decoded)
        self.assertIn({'has_precipitation':True,'temperature':0.5,'downfall':0.5,'temperature_modifier':None},decoded)
        self.assertTrue(any('1.000000059604644775390626' in f['json'] for f in fixtures))
        self.assertTrue(any('1e1000' in f['json'] for f in fixtures))
        self.assertTrue(any('"temperature":0.2,"temperature":null' in f['json'] for f in fixtures))


if __name__=='__main__':unittest.main()
