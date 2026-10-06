import itertools
import json
import unittest
from tools import test_anchor_parity as anchor


class AnchorCorpusTests(unittest.TestCase):
    def test_exclusive_alternatives_and_unrestricted_runtime_bounds(self):
        fixtures=list(anchor.corpus())
        self.assertEqual(fixtures,list(anchor.corpus()))
        self.assertEqual(len(fixtures),192)
        self.assertEqual(sum('generator' in f for f in fixtures),96)
        self.assertIn(-2**31,fixtures[0]['values'])
        self.assertIn(2**31-1,fixtures[0]['values'])
        decoded=[json.loads(s) for s in fixtures[0]['json']]
        names=['absolute','above_bottom','below_top','relative_to_sea_level']
        for combination in itertools.product([None,0,2032,'bad'],repeat=4):
            self.assertIn({k:v for k,v in zip(names,combination) if v is not None},decoded)
        self.assertIn('{"absolute":1,"absolute":null}',fixtures[0]['json'])


if __name__=='__main__':unittest.main()
