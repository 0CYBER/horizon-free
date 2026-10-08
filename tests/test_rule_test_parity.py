import json
import unittest
from tools import test_rule_test_parity as rules


class RuleTestCorpusTests(unittest.TestCase):
    def test_registry_coverage_logic_and_random_modes(self):
        facts = {'states': ['air', 'stone'], 'defaults': {'minecraft:air': 0, 'minecraft:stone': 1}}
        fixtures = list(rules.corpus(facts, {'minecraft:test': '{}'}))
        kinds = {json.loads(f['definition']).get('predicate_type') for f in fixtures if isinstance(json.loads(f['definition']), dict)}
        self.assertTrue({'always_true', 'not', 'all_of', 'any_of', 'block_match', 'blockstate_match', 'tag_match', 'height_match', 'random_block_match', 'random_blockstate_match'} <= kinds)
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertTrue(any('minecraft:test' in f['definition'] for f in fixtures))
        self.assertTrue(any('horizon:missing' in f['definition'] for f in fixtures))
        self.assertTrue(all(0 <= s < len(facts['states']) for f in fixtures for s in f['states']))


if __name__ == '__main__':
    unittest.main()
