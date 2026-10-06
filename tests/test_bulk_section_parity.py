import unittest
from tools import test_bulk_section_parity as bulk


class BulkSectionCorpusTests(unittest.TestCase):
    def test_all_state_cycles_cache_aliases_and_vertical_bounds(self):
        facts = {'states': list(range(5)), 'defaults': {'minecraft:air': 0, 'minecraft:cave_air': 1, 'minecraft:void_air': 2, 'minecraft:stone': 3, 'minecraft:water': 4}}
        fixtures = list(bulk.corpus(facts))
        self.assertEqual(fixtures, list(bulk.corpus(facts)))
        self.assertEqual({f['min'] for f in fixtures}, {-2032, -64, 0, 1968})
        self.assertEqual({f['prime'] for f in fixtures}, {False, True})
        self.assertTrue(all([4194304, 0] in f['chunks'] for f in fixtures))
        self.assertTrue(all(['reset'] in f['ops'] for f in fixtures))
        for f in fixtures:
            self.assertTrue(any(op[:2] == ['get', 67108864] for op in f['ops']))
            if f['min'] == -64:
                inserted = {op[4] for op in f['ops'] if op[:4] == ['set', 0, -63, 0]}
                self.assertEqual(inserted, set(range(5)))


if __name__ == '__main__':
    unittest.main()
