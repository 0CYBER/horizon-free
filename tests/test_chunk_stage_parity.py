import unittest
from tools import test_chunk_stage_parity as stages


class ChunkStageCorpusTests(unittest.TestCase):
    def test_all_stages_state_writes_and_reverse_transitions(self):
        names = ['air', 'cave_air', 'void_air', 'stone', 'water', 'lava', 'oak_leaves', 'oak_slab', 'seagrass', 'snow', 'powder_snow', 'glass', 'barrier']
        facts = {'states': names, 'defaults': {'minecraft:' + name: i for i, name in enumerate(names)}}
        fixtures = list(stages.corpus(facts))
        self.assertEqual(fixtures, list(stages.corpus(facts)))
        expected = {'empty', 'structure_starts', 'structure_references', 'biomes', 'terrain', 'features'}
        self.assertEqual({op[1] for f in fixtures for op in f['ops'] if op[0] == 'stage'}, expected)
        for f in fixtures[:2]:
            self.assertEqual({op[4] for op in f['ops'] if op[0] == 'set'}, set(range(len(names))))
        self.assertTrue(any(f['height'] == 4064 for f in fixtures))
        self.assertTrue(any(op[0] == 'set' and op[2] < f['min'] for f in fixtures for op in f['ops']))
        self.assertTrue(any(op[0] == 'set' and op[2] >= f['min'] + f['height'] for f in fixtures for op in f['ops']))
        self.assertTrue(any([op[1] for op in f['ops'] if op[0] == 'stage'][-1] == 'empty' for f in fixtures[2:]))


if __name__ == '__main__':
    unittest.main()
