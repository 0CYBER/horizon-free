import json
import unittest
from tools import test_material_stage_parity as stage


class MaterialStageCorpusTests(unittest.TestCase):
    def test_chunk_boundaries_repeated_builds_and_registry_coverage(self):
        names = ['stone', 'water', 'air', 'cave_air', 'void_air', 'oak_log',
                 'oak_leaves', 'netherrack', 'end_stone']
        facts = {'defaults': {'minecraft:' + name: i for i, name in enumerate(names)}}
        biomes = {'minecraft:plains': '', 'minecraft:eroded_badlands': '',
                  'minecraft:frozen_ocean': ''}
        fixtures = list(stage.corpus(facts, biomes))
        self.assertEqual(fixtures, list(stage.corpus(facts, biomes)))
        self.assertEqual(len(fixtures), 37)
        self.assertEqual(sum(f['repeats'] for f in fixtures), 50)
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertTrue(any(f['initial'] == [] for f in fixtures))
        self.assertTrue(any(f['possible'] is None for f in fixtures))
        self.assertTrue(any(f['write_min'] > f['min'] for f in fixtures))
        self.assertTrue(any(f['noise_min'] != f['min'] for f in fixtures))
        self.assertTrue(any(f['cx'] < 0 for f in fixtures))
        for f in fixtures:
            self.assertLessEqual(f['min'], f['write_min'])
            self.assertLess(f['write_max'], f['min'] + f['height'])
            for x, low, z, high, state in f['initial']:
                self.assertTrue(0 <= x < 16 and 0 <= z < 16)
                self.assertTrue(f['min'] <= low <= high < f['min'] + f['height'])
                self.assertIn(state, facts['defaults'].values())

    def test_generated_chunks_use_each_setting_and_share_the_router_surface_root(self):
        names = ['stone', 'water', 'air', 'cave_air', 'void_air', 'oak_log',
                 'oak_leaves', 'netherrack', 'end_stone']
        facts = {'defaults': {'minecraft:' + name: i for i, name in enumerate(names)}}
        settings = {'minecraft:overworld': json.dumps({
            'noise': {'min_y': -64, 'height': 384}, 'sea_level': 63,
            'default_block': 'minecraft:stone', 'default_fluid': 'minecraft:water',
            'material_rule': 'minecraft:overworld',
            'noise_router': {'chunk_surface_level': {'type': 'constant', 'argument': 64}},
            'aquifers': {'barrier': 0}})}
        generated = [f for f in stage.corpus(facts, {'minecraft:plains': ''}, settings) if 'router' in f]
        self.assertEqual(len(generated), 6)
        self.assertEqual(sum(f['repeats'] for f in generated), 8)
        self.assertEqual({f['legacy'] for f in generated}, {False, True})
        self.assertTrue(any(abs(f['cx']) > 1_000_000 for f in generated))
        for f in generated:
            self.assertEqual(f['preliminary'], f['router']['chunk_surface_level'])
            self.assertEqual(f['initial'], [])
            self.assertEqual(f['aquifers'], {'barrier': 0})
            self.assertTrue(f['biome_zoom'])

    def test_carving_masks_have_disjoint_runs_and_surface_restoration_inputs(self):
        names = ['grass_block', 'mycelium', 'dirt', 'bedrock']
        facts = {'defaults': {'minecraft:' + name: i for i, name in enumerate(names)}}
        f = {'min': -64, 'height': 384, 'sea': 63}
        result = stage.carving_fixture(f, facts)
        self.assertEqual(result['filters'], [0, 1])
        self.assertEqual({q[3] for q in result['initial']}, set(range(4)))
        runs = [q for q in result['spans'] if q[:2] == [6, 0]]
        self.assertEqual(len(runs), 2)
        self.assertLess(runs[1][3] + 1, runs[0][2])
        for x, z, low, high in result['spans']:
            self.assertTrue(0 <= x <= 15 and 0 <= z <= 15)
            self.assertTrue(-63 <= low <= high <= 312)

    def test_top_queries_cover_water_and_gradient_boundaries(self):
        f = {'cx': -1, 'cz': 1875000, 'min': -64, 'height': 384, 'sea': 63}
        queries = stage.top_queries(f)
        self.assertEqual(len(queries), 96)
        self.assertEqual({q[3] for q in queries}, {False, True})
        self.assertEqual({q[1] for q in queries}, {-65, -64, 62, 63, 319, 320})
        self.assertTrue(any(q[0] == -16 and q[2] == 30000000 for q in queries))
        self.assertTrue(any(q[0] == -1 and q[2] == 30000015 for q in queries))

    def test_generated_neighborhood_uses_preset_possible_biomes(self):
        names = ['stone', 'water', 'air', 'cave_air', 'void_air', 'oak_log', 'oak_leaves', 'netherrack', 'end_stone']
        facts = {'defaults': {'minecraft:' + name: i for i, name in enumerate(names)}}
        setting = {'noise': {'min_y': 0, 'height': 128}, 'sea_level': 63,
                   'default_block': 'minecraft:stone', 'default_fluid': 'minecraft:water',
                   'material_rule': 'minecraft:end', 'noise_router': {'chunk_surface_level': 0}}
        presets = {'minecraft:overworld': json.dumps([{'biome': 'minecraft:plains'}])}
        fixtures = list(stage.corpus(facts, {'minecraft:plains': ''}, {'minecraft:end': json.dumps(setting)}, presets))
        generated = [f for f in fixtures if 'biome_source' in f]
        self.assertEqual(len(generated), 6)
        self.assertTrue(all(f['biome_source'] == 'end' for f in generated))
        for f in generated:
            if f['legacy']:
                self.assertEqual(len(f['possible']), 5)
                self.assertIn('minecraft:small_end_islands', f['possible'])
            else:
                self.assertIsNone(f['possible'])


if __name__ == '__main__':
    unittest.main()
