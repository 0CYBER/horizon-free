import unittest
from tools import test_entity_obstruction_parity as obstruction


class EntityObstructionCorpusTests(unittest.TestCase):
    def test_shapes_flags_vehicle_chains_and_world_failures(self):
        fixtures=list(obstruction.corpus())
        self.assertEqual(fixtures,list(obstruction.corpus()))
        self.assertTrue(any(not f['shape'] and f['fail'] for f in fixtures))
        self.assertTrue(any(len(f['shape'])>1 for f in fixtures))
        self.assertTrue(any(f['exclude']>=0 for f in fixtures))
        flags={(e['building'],e['removed'],e['spectator']) for f in fixtures for e in f['entities']}
        self.assertEqual(len(flags),8)
        self.assertTrue(any(any(e['parent']>=0 for e in f['entities']) for f in fixtures))
        for f in fixtures:
            for i,e in enumerate(f['entities']):self.assertTrue(-1<=e['parent']<i)


if __name__=='__main__':unittest.main()
