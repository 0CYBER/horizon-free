import unittest
from tools import test_dynamic_support_parity as support


class DynamicSupportCorpusTests(unittest.TestCase):
    def test_entity_states_coordinate_boundaries_and_failures(self):
        facts={'states':[{'id':name} for name in ['minecraft:bamboo','minecraft:shulker_box','minecraft:moving_piston','minecraft:stone']], 'defaults':{'minecraft:stone':3}}
        fixtures=list(support.corpus(facts,[0,1,2]))
        self.assertEqual(fixtures,list(support.corpus(facts,[0,1,2])))
        self.assertEqual({f['entity'] for f in fixtures if f['state']==0},{'none'})
        self.assertEqual({f['entity'] for f in fixtures if f['state']==1},{'none','other','closed','opening','opened','closing'})
        self.assertEqual({f['entity'] for f in fixtures if f['state']==2},{'none','other','piston'})
        self.assertEqual({f['fail'] for f in fixtures},{-1,0,1,5,18})
        self.assertTrue(any(2147483647 in f['position'] for f in fixtures))
        self.assertTrue(any(-2147483648 in f['position'] for f in fixtures))


    def test_collision_progress_corpus_preserves_support_cases(self):
        facts={'states':[{'id':name} for name in ['minecraft:bamboo','minecraft:shulker_box']], 'defaults':{}}
        original=list(support.corpus(facts,[0,1]))
        fixtures=list(support.collision_corpus(facts,[0,1]))
        self.assertEqual([f for f in fixtures if not f.get('collision')],original)
        self.assertTrue(all(f.get('collision') for f in fixtures if 'shulker_progress' in f))
        progress={tuple(f['shulker_progress']) for f in fixtures if 'shulker_progress' in f}
        self.assertIn(('00000000','3f800000'),progress)
        self.assertIn(('3f800000','00000000'),progress)
        self.assertIn(('80000000','00000001'),progress)
        self.assertGreater(len(progress),2000)


if __name__=='__main__':unittest.main()
