"""The verify-static unittest suite exercises the book's computed totals."""
import unittest
from tools.neoforge_extension_counts import counts, render


class ExtensionCountsTest(unittest.TestCase):
    def test_rows_and_state_exclusions(self):
        source = '''## `IBlockExtension`
| `one()` | Covered | answer |
| `two()` | Partial | answer |
| `three()` | Missing | answer |
| `four()` | n/a | answer |
## `IBlockStateExtension`
| `one()` | Covered (only block) |
## `IItemExtension`
| `item()` | Missing | answer |
'''
        result = counts(source)
        self.assertEqual(dict(result['IBlockStateExtension']),
                         {'Covered': 0, 'Partial': 1, 'Missing': 1, 'n/a': 1})
        self.assertIn('| `IBlockExtension` | 4 | 1 | 1 | 1 | 1 |',
                      render('<!-- extension-counts -->\n' + source))
        self.assertEqual(dict(result['IItemExtension']), {'Missing': 1})
