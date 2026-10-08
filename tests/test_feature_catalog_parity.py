import unittest
from tools import test_feature_catalog_parity as catalog


class FeatureCatalogCorpusTests(unittest.TestCase):
    def test_named_roots_tag_shapes_and_mutation_failures(self):
        resources = catalog.catalog()
        self.assertEqual(resources, catalog.catalog())
        self.assertEqual(len(resources['features']), 10)
        self.assertEqual(len(resources['placed']), 15)
        self.assertEqual(len(resources['tags']), 5)
        fixtures = list(catalog.corpus())
        self.assertEqual({f['placed_root'] for f in fixtures}, {False, True})
        self.assertEqual({f['worldgen'] for f in fixtures}, {False, True})
        self.assertEqual({f['legacy'] for f in fixtures}, {False, True})
        self.assertTrue(any('#horizon:nested' in f['definition'] for f in fixtures))
        self.assertTrue(any('#horizon:empty' in f['definition'] for f in fixtures))
        self.assertTrue(any('#horizon:invalid' in f['definition'] for f in fixtures))
        self.assertEqual({f['fail'] for f in fixtures}, {-1, 0, 7})
        self.assertEqual({f['reject'] for f in fixtures}, {0, 3})


if __name__ == '__main__':
    unittest.main()
