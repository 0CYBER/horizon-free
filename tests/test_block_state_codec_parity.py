import json
import unittest
from tools import test_block_state_codec_parity as block


class BlockStateCodecCorpusTests(unittest.TestCase):
    def test_modes_property_defaults_and_integer_boundaries(self):
        facts={'states':[{'id':'minecraft:stone'},{'id':'minecraft:water','properties':{'level':'0'}}], 'defaults':{'minecraft:stone':0,'minecraft:water':1},'integers':[['minecraft:water','level']]}
        fixtures=list(block.corpus(facts))
        self.assertEqual(fixtures,list(block.corpus(facts)))
        self.assertEqual({f[0] for f in fixtures},{False,True})
        values=[json.loads(f[1]) for f in fixtures]
        for value in ['+01','-2147483649','\u0660\u0661','\uff10\uff11','\U0001d7d9']:
            self.assertIn({'id':'minecraft:water','properties':{'level':value}},values)
        self.assertIn({'id':'minecraft:stone','properties':False},values)
        self.assertTrue(any('"level":"0","level":"1"' in f[1] for f in fixtures))


if __name__=='__main__':unittest.main()
