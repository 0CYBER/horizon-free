import json
import unittest
from tools import test_replace_block_parity as replace


class ReplaceBlockCorpusTests(unittest.TestCase):
    def test_ordered_targets_failures_and_repeated_world_state(self):
        fixtures = list(replace.corpus({}))
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 1, 2, 4})
        self.assertEqual({f['reject'] for f in fixtures}, {0, 2})
        self.assertTrue(all(f['repeats'] == 3 for f in fixtures))
        definitions = [json.loads(f['definition']) for f in fixtures]
        self.assertTrue(any(d['targets'] == [] for d in definitions))
        self.assertTrue(any(isinstance(d['targets'], list) and len(d['targets']) == 3 for d in definitions))
        self.assertTrue(any('random_block_match' in f['definition'] for f in fixtures))


if __name__ == '__main__':
    unittest.main()
