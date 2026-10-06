import unittest
from tools import test_biome_chunk_parity as chunk


class BiomeChunkCorpusTests(unittest.TestCase):
    def test_bounds_section_failure_and_query_coverage(self):
        fixtures = list(chunk.corpus())
        self.assertEqual(fixtures, list(chunk.corpus()))
        self.assertEqual(len(fixtures), 20)
        self.assertTrue(any(f['height'] == 4064 for f in fixtures))
        self.assertTrue(any(f['cx'] == -(2**31) for f in fixtures))
        for f in fixtures:
            self.assertEqual({op[2] for op in f['ops']}, {-1, 0, 63, 64, 65})
            self.assertTrue(any(op[3] % 16 != 0 for op in f['ops']))
            for op in f['ops']:
                self.assertTrue(f['min'] <= op[3] <= op[4] < f['min'] + f['height'])
                self.assertTrue(1 <= op[1] <= 512)
            for q in f['queries']:
                self.assertTrue(all(-(2**31) <= v < 2**31 for v in q))


if __name__ == '__main__':
    unittest.main()
