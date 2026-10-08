import json
import unittest
from tools import test_biome_generation_parity as generation


class BiomeGenerationCorpusTests(unittest.TestCase):
    def test_router_preset_seed_and_clipped_generation_coverage(self):
        bundle = {'settings': {'minecraft:overworld': json.dumps({'noise': {'min_y': -64, 'height': 384}})},
                  'presets': {'minecraft:overworld': '[]', 'minecraft:nether': '[]'}}
        all_fixtures = list(generation.corpus(bundle))
        self.assertEqual({f.get('source', 'multi') for f in all_fixtures}, {'multi', 'fixed', 'checkerboard', 'end'})
        fixtures = [f for f in all_fixtures if 'source' not in f and not f.get('retrogen')]
        self.assertEqual(all_fixtures, list(generation.corpus(bundle)))
        self.assertEqual(len(fixtures), 12)
        self.assertEqual(sum(f['repeats'] for f in fixtures), 16)
        self.assertEqual({f['preset'] for f in fixtures}, set(bundle['presets']))
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertTrue(any(f['write_min'] > f['min'] for f in fixtures))
        self.assertTrue(any(abs(f['cx']) > 1_000_000 for f in fixtures))
        for f in fixtures:
            self.assertEqual(len(f['queries']), 180)
            self.assertTrue(any(q[1] < f['min'] for q in f['queries']))
            self.assertTrue(any(q[1] >= f['min'] + f['height'] for q in f['queries']))

    def test_upgrade_height_all_sources_and_retained_cave_candidates(self):
        bundle = {'settings': {
            'minecraft:overworld': json.dumps({'noise': {'min_y': -64, 'height': 384}}),
            'minecraft:end': json.dumps({'noise': {'min_y': 0, 'height': 128}})},
            'presets': {'minecraft:overworld': '[]', 'minecraft:nether': '[]'}}
        fixtures = [f for f in generation.corpus(bundle) if f.get('retrogen')]
        self.assertEqual(len(fixtures), 36)
        self.assertEqual({f['setting'] for f in fixtures}, {'minecraft:overworld'})
        self.assertEqual({f.get('source', 'multi') for f in fixtures}, {'multi', 'fixed', 'checkerboard', 'end'})
        self.assertEqual({f['fixed_biome'] for f in fixtures if 'fixed_biome' in f},
                         {'minecraft:lush_caves', 'minecraft:dripstone_caves', 'minecraft:deep_dark'})
        self.assertTrue(all((f['write_min'], f['write_max'], f['repeats']) == (-64, -1, 2) for f in fixtures))
        self.assertTrue(all(any(q[1] == 0 for q in f['queries']) for f in fixtures))


if __name__ == '__main__':
    unittest.main()
