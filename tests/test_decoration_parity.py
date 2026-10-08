import unittest
from tools import test_decoration_parity as decoration


class DecorationCorpusTests(unittest.TestCase):
    def test_global_indices_foreign_palettes_failures_and_reproducibility(self):
        resources = {'a': [[], ['horizon:a', 'horizon:b']], 'b': [['horizon:c'], ['horizon:b']]}
        fixtures = list(decoration.corpus(resources))
        self.assertEqual(fixtures, list(decoration.corpus(resources)))
        self.assertTrue(any(f['biomes'] == [[[0, 1]], [[1, 0]]] for f in fixtures))
        self.assertTrue(any(len(b) > 12 for f in fixtures for b in f['biomes']))
        self.assertTrue(any(f['palettes'][:8] == [[3]] * 8 and f['palettes'][8] == [1] for f in fixtures))
        for key in ['feature_fail', 'query_fail', 'chunk_fail', 'notify_fail']:
            self.assertTrue(any(f[key] >= 0 for f in fixtures))
        for f in fixtures:
            self.assertEqual(len(f['palettes']), 9)
            self.assertTrue(all(0 <= b <= len(f['biomes']) for p in f['palettes'] for b in p))
            self.assertTrue(all(0 <= feature < len(f['placements']) for b in f['biomes'] for step in b for feature in step))


if __name__ == '__main__':
    unittest.main()
