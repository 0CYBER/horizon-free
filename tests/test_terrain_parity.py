import json
import unittest
from tools import test_terrain_parity as terrain


class TerrainCorpusTests(unittest.TestCase):
    def inputs(self):
        names=["air","void_air","cave_air","stone","water","lava","oak_leaves","glass","oak_slab"]
        facts={"states":[["minecraft:"+name,name.endswith("air"),name in ["water","lava","oak_slab"]] for name in names],
               "defaults":{"minecraft:"+name:i for i,name in enumerate(names)}}
        setting={"noise":{"min_y":-64,"height":384},"default_block":"minecraft:stone","default_fluid":"minecraft:water","sea_level":63,"noise_router":{},"aquifers":{"barrier":0}}
        return facts,{"settings":{"minecraft:overworld":json.dumps(setting)}}

    def test_corpus_retains_presets_seed_modes_and_repeat_fills(self):
        facts,bundle=self.inputs();fixtures=list(terrain.corpus(facts,bundle))
        self.assertEqual(fixtures,list(terrain.corpus(facts,bundle)))
        self.assertEqual(len(fixtures),66)
        self.assertEqual({f["legacy"] for f in fixtures[:6]},{True,False})
        self.assertEqual({f["seed"] for f in fixtures[:6]},{0,-1,0x123456789abcdef})
        self.assertTrue(all(f["initial"] for f in fixtures))
        self.assertTrue(any(f["repeats"]==2 for f in fixtures[:6]))
        self.assertTrue(any(f["aquifers"] is None for f in fixtures))
        self.assertTrue(any(f["aquifers"] is not None for f in fixtures))

    def test_partial_fills_and_input_writes_stay_inside_chunk(self):
        facts,bundle=self.inputs();fixtures=list(terrain.corpus(facts,bundle))
        self.assertTrue(any(f["min"]<f["noise_min"] for f in fixtures))
        for f in fixtures:
            self.assertLessEqual(f["min"],f["noise_min"])
            self.assertLessEqual(f["noise_min"]+f["noise_height"],f["min"]+f["height"])
            for x,y,z,state in f["initial"]:
                self.assertTrue(0<=x<16 and 0<=z<16)
                self.assertTrue(f["min"]<=y<f["min"]+f["height"])
                self.assertTrue(0<=state<len(facts["states"]))


if __name__=="__main__":unittest.main()
