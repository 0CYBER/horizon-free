import json
import unittest
from tools import test_biome_source_codec_parity as codec


class BiomeSourceCodecCorpusTests(unittest.TestCase):
    def test_registry_holders_fallbacks_and_numeric_boundaries(self):
        bundle = {'biomes': {'minecraft:plains': ''}, 'biome_tags': {'minecraft:is_overworld': ''}}
        fixtures = list(codec.corpus(bundle))
        self.assertEqual(fixtures, list(codec.corpus(bundle)))
        values = [json.loads(f) for f in fixtures]
        objects = [v for v in values if isinstance(v, dict)]
        self.assertTrue({'fixed', 'checkerboard', 'the_end', 'multi_noise'} <= {v.get('type') for v in objects})
        self.assertTrue(any(v.get('biomes') == '#minecraft:is_overworld' for v in objects))
        self.assertTrue(any(v.get('biomes') == [] and v.get('preset') == 'overworld' for v in objects))
        self.assertTrue(any(v.get('preset') == {'preset': 'nether'} for v in objects))
        self.assertTrue(any('4294967298' in f for f in fixtures))
        self.assertTrue(any('"scale":-1,"scale":2' in f for f in fixtures))
        self.assertEqual(len(codec.positions()), 7)


if __name__ == '__main__':
    unittest.main()
