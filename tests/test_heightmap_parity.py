import unittest
from tools import test_heightmap_parity as heightmap


class HeightmapCorpusTests(unittest.TestCase):
    def facts(self):
        names=["air","cave_air","void_air","stone","water","lava","oak_leaves","oak_slab","seagrass","snow","powder_snow","glass","barrier"]
        return {"states":[["minecraft:"+name,name.endswith("air"),name in ["water","lava"],name] for name in names],
                "defaults":{"minecraft:"+name:i for i,name in enumerate(names)}}

    def test_every_state_is_inserted_queried_and_removed(self):
        facts=self.facts();fixtures=list(heightmap.corpus(facts))
        self.assertEqual(fixtures,list(heightmap.corpus(facts)))
        ops=fixtures[0]["ops"]
        for i in range(len(facts["states"])):
            self.assertEqual(ops[i*3],["set",4,5,7,i,63])
            self.assertEqual(ops[i*3+1],["query",4,7])
            self.assertEqual(ops[i*3+2],["set",4,5,7,facts["defaults"]["minecraft:air"],63])

    def test_corpus_keeps_vertical_edges_raw_fallbacks_and_traces(self):
        fixtures=list(heightmap.corpus(self.facts()))
        self.assertIn((-2032,4064),{(f["min"],f["height"]) for f in fixtures})
        for f in fixtures:
            self.assertEqual(f["min"]%16,0)
            self.assertEqual(f["height"]%16,0)
            self.assertGreaterEqual(f["min"],-2032)
            self.assertLessEqual(f["min"]+f["height"],2032)
            for op in f["ops"]:
                if op[0]=="set":self.assertTrue(f["min"]<=op[2]<f["min"]+f["height"])
        ops=[op for f in fixtures for op in f["ops"]]
        self.assertTrue(any(op[0]=="raw" and not op[2] for op in ops))
        self.assertTrue(any(op[0]=="raw" and "ffffffffffffffff" in op[2] for op in ops))
        self.assertIn(["prime",0],ops)
        self.assertIn(["query",-1,1],ops)
        self.assertIn(["query",0,16],ops)
        self.assertTrue(any(f["trace"] for f in fixtures))


if __name__=="__main__":unittest.main()
