import unittest
from tools import test_decoration_region_parity as region


class DecorationRegionCorpusTests(unittest.TestCase):
    def test_real_features_mixed_biomes_live_heights_and_partial_writes(self):
        fixtures = list(region.corpus({}))
        self.assertEqual(fixtures, list(region.corpus({})))
        self.assertEqual(len(fixtures), 240)
        self.assertEqual({f['stage'] for f in fixtures}, {'empty', 'biomes', 'terrain'})
        self.assertEqual({f['write_fail'] for f in fixtures}, {-1, 7})
        self.assertTrue(any(f['palettes'] == [[0]] * 9 for f in fixtures))
        self.assertTrue(any(3 in p for f in fixtures for p in f['palettes']))
        for f in fixtures:
            self.assertEqual(len(f['features']), 6)
            self.assertEqual(len(f['placements']), len(f['features']))
            self.assertTrue(all(any(m['type'] == 'biome' for m in p) for p in f['placements']))
            self.assertTrue(all(0 <= feature < len(f['features']) for biome in f['biomes'] for step in biome for feature in step))


if __name__ == '__main__':
    unittest.main()
