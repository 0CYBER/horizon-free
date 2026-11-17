"""Regression: test-support state needs a single, direct test/testing-only cfg.

Rust ANDs stacked ``#[cfg]`` attributes, so a conjunction such as
``#[cfg(not(test))] #[cfg(feature = "testing")]`` still holds in an ordinary
build with that feature. It must not be admitted as absent from production just
because one half of the stack names the testing feature.
"""
import sys
from pathlib import Path
import unittest
from unittest import mock

# Append tools/ to sys.path only for the import (never at sys.path[0]); that is
# how check_mod_vocabulary_state resolves its `layering.rust_text` helper.
with mock.patch.object(sys, 'path', [*sys.path, str(Path(__file__).resolve().parents[1] / 'tools')]):
    from check_mod_vocabulary_state import violations

STATE = 'static STATE: AtomicBool = AtomicBool::new(false);'
ALLOWED = {'fixture.rs::STATE': {'type': 'AtomicBool', 'category': 'test-support'}}
MESSAGE = 'test-support state requires a direct test/testing-only cfg'


class TestSupportCfgRegression(unittest.TestCase):
    def errors(self, source):
        return violations({'fixture.rs': source}, ALLOWED)

    def assert_rejected(self, source):
        errors = self.errors(source)
        self.assertTrue(any(MESSAGE in error for error in errors), errors)

    def test_direct_test_only_cfgs_are_admitted(self):
        # These are the approved single-attribute forms; none is gated by an
        # additional predicate, so the item really is absent from production.
        for cfg in ('#[cfg(feature = "testing")]',
                    '#[cfg(any(test, feature = "testing"))]',
                    '#[cfg(any(feature = "testing", test))]'):
            with self.subTest(cfg=cfg):
                self.assertEqual(self.errors(cfg + '\n' + STATE), [])

    def test_stacked_negated_test_and_testing_feature_is_rejected(self):
        # The exact defect: any() saw the approved feature half and admitted a
        # state that exists in a normal build with the testing feature.
        self.assert_rejected('#[cfg(not(test))]\n#[cfg(feature = "testing")]\n' + STATE)

    def test_other_stacked_or_compound_cfg_forms_are_rejected(self):
        for cfg in (
            '#[cfg(feature = "testing")]\n#[cfg(not(test))]',
            '#[cfg(feature = "testing")]\n#[cfg(feature = "other")]',
            '#[cfg(any(test, feature = "testing"))]\n#[cfg(feature = "extra")]',
            '#[cfg(all(feature = "testing", not(test)))]',
        ):
            with self.subTest(cfg=cfg):
                self.assert_rejected(cfg + '\n' + STATE)

    def test_unrelated_attributes_do_not_hide_a_direct_test_cfg(self):
        source = '#[allow(dead_code)]\n#[cfg(feature = "testing")]\n' + STATE
        self.assertEqual(self.errors(source), [])


if __name__ == '__main__':
    unittest.main()
