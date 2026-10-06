import unittest
import json
from tools import test_material_condition_parity as material


class MaterialConditionCorpusTests(unittest.TestCase):
    def test_numeric_conditions_cache_updates_and_named_streams(self):
        fixtures=list(material.corpus())
        self.assertEqual(fixtures,list(material.corpus()))
        self.assertEqual(len(fixtures),48)
        self.assertEqual({f['legacy'] for f in fixtures},{False,True})
        conditions=fixtures[0]['conditions']
        self.assertEqual({c[0] for c in conditions},{'hole','steep','y','water','stone','gradient','not'})
        self.assertEqual({c[1][0] for c in conditions if c[0]=='y'},set(range(4)))
        self.assertEqual({c[4] for c in conditions if c[0]=='stone'},{False,True})
        gradients=[c for c in conditions if c[0]=='gradient']
        self.assertTrue(any(c[2]==c[3] for c in gradients))
        self.assertTrue(any(c[2]==[0,64] and c[3]==[0,-64] for c in gradients))
        ops=fixtures[0]['ops']
        self.assertEqual(ops[:2],[['eval'],['eval']])
        self.assertTrue(any(op[0]=='add' for op in ops))
        self.assertTrue({'7f800000','ff800000','7fc00000'}<={op[2] for op in ops if op[0]=='input'})

    def test_codec_mutations_and_named_references(self):
        definitions={"horizon:forward":'{"type":"not","invert":"horizon:leaf"}',"horizon:leaf":'{"type":"hole"}'}
        fixtures=list(material.codec_corpus(definitions))
        self.assertEqual(len(fixtures),8)
        self.assertEqual(fixtures,list(material.codec_corpus(definitions)))
        decoded=[json.loads(s) for s in fixtures[0]['json']]
        self.assertIn('horizon:forward',decoded)
        self.assertIn({'type':'not','invert':'horizon:forward'},decoded)
        self.assertIn({'type':'water','offset':0,'surface_depth_multiplier':21,'add_stone_depth':False},decoded)
        self.assertIn({'type':'water','offset':0,'surface_depth_multiplier':0,'add_stone_depth':None},decoded)
        self.assertTrue(any('4294967296' in s for s in fixtures[0]['json']))
        self.assertIn('{"type":"hole","type":null}',fixtures[0]['json'])

    def test_rule_composition_and_independent_application_positions(self):
        fixtures=list(material.rule_corpus({"horizon:test":'{"type":"hole"}'}))
        self.assertEqual(fixtures,list(material.rule_corpus({"horizon:test":'{"type":"hole"}'})))
        self.assertEqual(len(fixtures),24)
        rules=fixtures[0]['rules']
        self.assertIn(['sequence',[]],rules)
        self.assertIn(['sequence',[['block',8]]],rules)
        self.assertIn(['band'],rules)
        self.assertTrue(any(op[0]=='rule' for op in fixtures[0]['ops']))
        ops=fixtures[0]['ops']
        self.assertEqual(ops[:2],[['apply',0,0,0],['apply',0,0,0]])
        self.assertTrue(any(ops[i][0]=='apply' and ops[i+1][0]=='apply' and ops[i][1:]!=ops[i+1][1:] for i in range(len(ops)-1)))

    def test_noise_supplier_modes_and_nonfinite_thresholds(self):
        fixtures=list(material.noise_corpus({'minecraft:surface':'unused'}))
        self.assertEqual(len(fixtures),24)
        self.assertEqual(fixtures,list(material.noise_corpus({'minecraft:surface':'unused'})))
        self.assertEqual({(f['legacy'],f['real_noise']) for f in fixtures},{(False,False),(False,True),(True,False),(True,True)})
        fake=next(f for f in fixtures if not f['real_noise'])
        thresholds={b for c in fake['conditions'] if c[0]=='noise' for b in c[2:4]}
        self.assertTrue({'7ff0000000000000','fff0000000000000','7ff8000000000000'}<=thresholds)
        self.assertEqual({c[4] for c in fake['conditions'] if c[0]=='noise'},{False,True})
        self.assertEqual(fake['ops'][:2],[['eval'],['eval']])
        self.assertTrue(any(op[0]=='add' for op in fake['ops']))

    def test_biome_folding_null_results_and_failed_retry_sequences(self):
        resources={'minecraft:plains':'unused','minecraft:desert':'unused'}
        fixtures=list(material.biome_corpus(resources))
        self.assertEqual(fixtures,list(material.biome_corpus(resources)))
        self.assertEqual(len(fixtures),18)
        self.assertTrue(any(f['possible_biomes'] is None for f in fixtures))
        self.assertTrue(any(f['possible_biomes']==[] for f in fixtures))
        self.assertIn(['temperature'],fixtures[0]['conditions'])
        self.assertIn(['biome',[]],fixtures[0]['conditions'])
        self.assertIn(['biome','#throw'],fixtures[0]['ops'])
        self.assertIn(['biome',None],fixtures[0]['ops'])
        self.assertTrue(any(op[0]=='add' for op in fixtures[0]['ops']))

    def test_biome_tag_aliases_and_rule_application(self):
        biomes={'minecraft:plains':'unused','minecraft:badlands':'unused'}
        tags={'minecraft:is_badlands':'unused','horizon:empty':'unused'}
        fixtures=list(material.biome_tag_corpus(biomes,tags))
        self.assertEqual(fixtures,list(material.biome_tag_corpus(biomes,tags)))
        self.assertEqual(len(fixtures),5)
        decoded=[json.loads(value) for value in fixtures[0]['json']]
        self.assertIn({'type':'biome','biome_is':'#is_badlands'},decoded)
        self.assertIn({'type':'biome','biome_is':'#horizon:empty'},decoded)
        self.assertIn({'type':'biome','biome_is':['#minecraft:is_badlands']},decoded)
        self.assertTrue(fixtures[0]['rules'])
        self.assertTrue(any(op[0]=='apply' for op in fixtures[0]['ops']))
        self.assertIn(['biome',None],fixtures[0]['ops'])

    def test_preliminary_planes_failures_and_scalar_boundaries(self):
        fixtures=list(material.preliminary_corpus())
        self.assertEqual(fixtures,list(material.preliminary_corpus()))
        self.assertEqual(len(fixtures),61)
        self.assertTrue(any(f['volume'][6:]!=[1,1,1] for f in fixtures))
        self.assertEqual({f['ops'][0][2] for f in fixtures},{-1,0,1,3,19})
        self.assertIn(['density','ff800000',-1],fixtures[0]['ops'])
        self.assertIn(['density','7fc00000',-1],fixtures[0]['ops'])
        self.assertIn(['add',['above']],fixtures[0]['ops'])
        self.assertEqual(fixtures[-1]['ops'][1],['minimum'])

    def test_ore_sources_sampling_failures_and_random_modes(self):
        fixtures=list(material.ore_corpus())
        self.assertEqual(fixtures,list(material.ore_corpus()))
        self.assertEqual(len(fixtures),57)
        self.assertEqual({f['legacy'] for f in fixtures},{False,True})
        self.assertTrue(any('7fc00000' in f['ore_values'] for f in fixtures))
        self.assertTrue(any(f['volume'][6:]!=[1,1,1] for f in fixtures))
        self.assertEqual({f['ops'][0][1] for f in fixtures[-9:]},{0,1,2})
        self.assertEqual({f['ops'][0][3] for f in fixtures[-9:]},{0,1,7})
        self.assertTrue(any(op[0]=='rule' for op in fixtures[0]['ops']))


if __name__=='__main__':unittest.main()
