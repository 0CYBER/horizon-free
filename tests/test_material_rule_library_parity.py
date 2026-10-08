import json
import unittest
from tools import test_material_rule_library_parity as rules


class MaterialRuleLibraryCorpusTests(unittest.TestCase):
    def test_bundled_modes_and_codec_mutations(self):
        definitions={'minecraft:leaf':'{"type":"block","result_state":"minecraft:stone"}'}
        biomes={'minecraft:plains':'unused','minecraft:desert':'unused'}
        fixtures=list(rules.corpus(definitions,biomes))
        self.assertEqual(fixtures,list(rules.corpus(definitions,biomes)))
        self.assertEqual(len(fixtures),12)
        self.assertEqual({f['legacy'] for f in fixtures},{False,True})
        self.assertTrue(any(f['possible_biomes'] is None for f in fixtures))
        self.assertTrue(all(f['real_noise'] for f in fixtures))
        self.assertTrue(all(f['library_rules']==['minecraft:leaf'] for f in fixtures))
        values=[json.loads(v) for v in fixtures[0]['library_json']]
        self.assertIn({'type':'block','result_state':{'id':'minecraft:water','properties':{'level':'+01'}}},values)
        self.assertTrue(any(isinstance(v,dict) and v.get('raw_ore_chance')==1.01 for v in values))
        ops=fixtures[0]['ops']
        self.assertTrue(any(ops[i][0]=='apply' and ops[i+1][0]=='apply' and ops[i][1:]!=ops[i+1][1:] for i in range(len(ops)-1)))


if __name__=='__main__':unittest.main()
