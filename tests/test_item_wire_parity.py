"""Regression: item-wire parity must account for every row the Rust probe emits.

compare() used to iterate only over vanilla rows, so a probe that emitted an
extra row (notably an ``error`` record) was silently accepted as a green pass.
These tests pin the label accounting in both directions.
"""
import unittest
from unittest import mock

from tools import test_item_wire_parity as tool


def _vanilla(label, added):
    return {'label': label, 'added': added, 'removed': [1]}


def _rust(label, added):
    return {'label': label, 'left': 0, 'added': added, 'removed': [1],
            'same_bytes': True, 'typed': False}


class CompareSymmetricLabelsTest(unittest.TestCase):
    def setUp(self):
        # Limit the component-id coverage check to the four ids used below so
        # it does not mask the label-key behaviour under test.
        patcher = mock.patch.object(tool, 'COMPONENT_TYPES', 4)
        patcher.start()
        self.addCleanup(patcher.stop)

    @staticmethod
    def _matching_rows():
        added = [0, 1, 2, 3]
        vanilla = [_vanilla(f'L{i}', added if i == 0 else [1]) for i in range(4)]
        rust = [_rust(f'L{i}', added if i == 0 else [1]) for i in range(4)]
        return vanilla, rust

    def test_extra_rust_rows_are_mismatches(self):
        vanilla, rust = self._matching_rows()
        rust.append({'label': 'GHOST-1', 'error': 'phantom error from a broken probe'})
        rust.append(_rust('GHOST-2', [1]))
        failures, stacks, covered = tool.compare(vanilla, rust)
        self.assertTrue(failures, 'extra Rust rows must not produce a green pass')
        joined = '\n'.join(failures)
        self.assertIn('GHOST-1', joined)
        self.assertIn('GHOST-2', joined)

    def test_exact_match_passes(self):
        vanilla, rust = self._matching_rows()
        failures, stacks, covered = tool.compare(vanilla, rust)
        self.assertEqual(failures, [])
        self.assertEqual(stacks, 4)

    def test_duplicate_rust_label_is_a_mismatch(self):
        vanilla, rust = self._matching_rows()
        rust.append(_rust('L0', [0, 1, 2, 3]))
        failures, stacks, covered = tool.compare(vanilla, rust)
        self.assertTrue(failures, 'a duplicate Rust row must be reported')
        self.assertIn('L0', '\n'.join(failures))

    def test_equal_duplicate_counts_are_mismatches(self):
        for first in (_rust('L0', [0, 1, 2, 3]), {'label': 'L0', 'error': 'early probe error'}):
            with self.subTest(first=first):
                vanilla, rust = self._matching_rows()
                vanilla.insert(0, _vanilla('L0', [0, 1, 2, 3]))
                rust.insert(0, first)
                failures, stacks, covered = tool.compare(vanilla, rust)
                self.assertTrue(failures, 'equal duplicate counts must not hide an earlier Rust row')
                self.assertIn('L0', '\n'.join(failures))
                self.assertEqual(covered, {0, 1, 2, 3})


if __name__ == '__main__':
    unittest.main()
