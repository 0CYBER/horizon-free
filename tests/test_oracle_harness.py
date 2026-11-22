"""Regression tests for the pure-Python parts of tools.oracle_harness."""
import tempfile
import unittest
from pathlib import Path

from tools import oracle_harness


class ExplainTests(unittest.TestCase):
    def test_rust_only_section_is_counted_and_shown(self):
        """explain() compares sections symmetrically, not just vanilla's."""
        with tempfile.TemporaryDirectory(prefix='hz-oracle-explain-') as tmp:
            build = Path(tmp)
            (build / 'client.txt').write_text('== A\nsame\n== B\nvanilla only\n', encoding='utf-8')
            (build / 'rust.txt').write_text('== A\nsame\n== C\nrust only\n', encoding='utf-8')
            report = oracle_harness.explain(build, 'mismatch')
        self.assertIn('2 differing cases.', report)
        self.assertIn('case B', report)
        self.assertIn('case C', report)
        self.assertNotIn('case A', report)


if __name__ == '__main__':
    unittest.main()
