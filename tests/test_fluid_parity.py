import unittest
from tools import test_fluid_parity as fluid


class FluidCorpusTests(unittest.TestCase):
    def test_liquid_levels_grate_sources_and_query_failures(self):
        facts={'states':[{'id':'minecraft:air'},{'id':'minecraft:water','properties':{'level':'0'}},{'id':'minecraft:water','properties':{'level':'8'}},{'id':'minecraft:lava','properties':{'level':'1'}},{'id':'minecraft:copper_grate','properties':{'waterlogged':'true'}},{'id':'minecraft:stone'}]}
        fixtures=list(fluid.corpus(facts))
        self.assertEqual(fixtures,list(fluid.corpus(facts)))
        self.assertEqual({v['state'] for v in fixtures},set(range(5)))
        self.assertEqual({v['above'] for v in fixtures},set(range(5)))
        self.assertEqual({v['sturdy'] for v in fixtures},{False,True})
        self.assertEqual({v['fail'] for v in fixtures},{-1,0,1,2})
        self.assertTrue(any(v['position'][1]==2147483647 for v in fixtures))


if __name__=='__main__':unittest.main()
