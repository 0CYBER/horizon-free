import json
import unittest
from tools import test_cave_carver_parity as cave


class CaveCarverCorpusTests(unittest.TestCase):
    def test_seeds_origin_neighborhoods_and_partial_failures(self):
        config={'type':'minecraft:cave','count':0,'probability':0.15,'thickness':1,'y':{'absolute':40},'floor_level':-0.7}
        resources={'cave':json.dumps(config),'nether_cave':json.dumps(config)}
        fixtures=list(cave.corpus(resources))
        self.assertEqual(fixtures,list(cave.corpus(resources)))
        self.assertEqual({f['legacy'] for f in fixtures},{False,True})
        self.assertEqual({f['fail'] for f in fixtures},{-1,0,1,63})
        self.assertTrue(any(f['target']==[-2**31,2**31-1] for f in fixtures))
        self.assertTrue(any(f['origin']==[8,-8] for f in fixtures))
        self.assertTrue(any(f['repeats']==2 for f in fixtures))
        definitions=[json.loads(f['definition']) for f in fixtures]
        self.assertTrue(any(isinstance(v,dict) and v.get('thickness')==6 for v in definitions))


if __name__=='__main__':unittest.main()
