import json
import unittest
from tools import test_state_provider_parity as provider


class StateProviderCorpusTests(unittest.TestCase):
    def test_exclusive_decode_empty_fallback_rotations_and_shared_streams(self):
        facts = {'states': [{'id': 'minecraft:stone'}, {'id': 'minecraft:water'}], 'defaults': {'minecraft:stone': 0, 'minecraft:water': 1}}
        resources = {'horizon:provider': '{"id":"stone"}'}
        fixtures = list(provider.corpus(facts, {'horizon:empty': '{"values":[]}'}, catalog_resources=resources, survival_rules={"minecraft:stone": "Always", "minecraft:water": "Always"}))
        self.assertEqual(fixtures, list(provider.corpus(facts, {'horizon:empty': '{"values":[]}'}, catalog_resources=resources, survival_rules={"minecraft:stone": "Always", "minecraft:water": "Always"})))
        definitions = [json.loads(f['definition']) for f in fixtures]
        self.assertTrue(any(isinstance(v, dict) and v.get('type') == 'simple' and v.get('id') == 'stone' for v in definitions))
        self.assertTrue(any(isinstance(v, dict) and v.get('type') == 'random_block' and v.get('blocks') == [] for v in definitions))
        self.assertEqual({f['typed'] for f in fixtures}, {False, True})
        self.assertEqual({(f['legacy'], f['worldgen']) for f in fixtures}, {(False, False), (True, False), (False, True), (True, True)})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 1, 2})
        for kind in ['copy_properties', 'randomized_int', 'noise', 'noise_threshold', 'dual_noise', 'rule_based']:
            self.assertTrue(any(isinstance(v, dict) and v.get('type') == kind for v in definitions))
        self.assertTrue(any([2147483647, -2147483648, 2147483647] in f.get('positions', []) for f in fixtures))
        self.assertEqual({f.get('query_fail', -1) for f in fixtures}, {-1, 0, 1, 2})
        self.assertTrue(any(f.get('bounds') == [2147483647, 2, 0] for f in fixtures))
        self.assertTrue(any(f.get("holder") and json.loads(f["definition"]) == "horizon:provider" for f in fixtures))
        conditions = [rule['if_true'] for v in definitions if isinstance(v, dict) and v.get('type') == 'rule_based' and isinstance(v.get('rules'), list) for rule in v['rules'] if isinstance(rule, dict) and isinstance(rule.get('if_true'), dict)]
        for kind in ['matching_block_tag', 'matching_blocks', 'matching_fluids', 'matching_biomes', 'unobstructed', 'solid', 'replaceable', 'has_sturdy_face', 'would_survive', 'height_range', 'volume_match']:
            self.assertTrue(any(v.get('type') == kind for v in conditions))
        for direction in ['down', 'up', 'north', 'south', 'west', 'east']:
            self.assertTrue(any(isinstance(v, dict) and v.get('type') == 'rotated' and v.get('direction') == direction for v in definitions))


if __name__ == '__main__':
    unittest.main()
