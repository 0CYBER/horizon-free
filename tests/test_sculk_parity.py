"""The sculk harness's comparator must be symmetric.

`tools/test_sculk_parity.py::compare()` recorded a difference for a scenario that
vanilla had and Horizon lacked, but ignored a scenario that only Horizon had. A
whole extra observed scenario therefore counted as parity. These tests load the
real `compare()` and pin both directions of that contract.
"""
import importlib.util
import sys
import types
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / 'tools' / 'test_sculk_parity.py'

# The harness imports Java-oracle plumbing at module scope; compare() needs none
# of it. Loading the file by path with those names stubbed keeps this a pure-Python
# regression (no jars, no Rust probes), while still exercising the shipped code.
_STUBS = {
    name: types.ModuleType(name)
    for name in ('fast_bins', 'lab', 'oracle_harness', 'test_rust_parity', 'test_model_baking_parity')
}
_spec = importlib.util.spec_from_file_location('test_sculk_parity_under_test', TOOL)
sculk = importlib.util.module_from_spec(_spec)
# A loaded tool may append to sys.path; run the import against a copy and discard
# any mutation so the test runner's import state is left untouched.
with mock.patch.dict(sys.modules, _STUBS), mock.patch.object(sys, 'path', sys.path.copy()):
    _spec.loader.exec_module(sculk)


class SculkCompareSymmetryTest(unittest.TestCase):
    def test_matched_scenarios_are_reported_and_counted_exactly_as_before(self):
        transcript = {'real': {0: ['same observation'], 1: ['second observation']}}
        report, clean = sculk.compare(transcript, {k: dict(v) for k, v in transcript.items()})
        self.assertEqual(clean, 1)
        self.assertEqual(report, ['real: all 2 ticks match'])

    def test_horizon_only_scenario_is_reported_and_not_counted_as_clean(self):
        vanilla = {'real': {0: ['same observation']}}
        rust = {
            'real': {0: ['same observation']},
            'ghost': {0: ['an observation only Horizon produced']},
        }
        report, clean = sculk.compare(vanilla, rust)
        self.assertTrue(any('ghost' in line for line in report), report)
        self.assertTrue(any('missing from vanilla' in line for line in report), report)
        self.assertEqual(clean, 1)
        self.assertLess(clean, len(rust))
        self.assertEqual(len(report), 2)

    def test_vanilla_only_scenario_is_still_reported(self):
        vanilla = {
            'real': {0: ['same observation']},
            'vonly': {0: ['an observation only vanilla produced']},
        }
        rust = {'real': {0: ['same observation']}}
        report, clean = sculk.compare(vanilla, rust)
        self.assertTrue(any('vonly' in line for line in report), report)
        self.assertEqual(clean, 1)


if __name__ == '__main__':
    unittest.main()
