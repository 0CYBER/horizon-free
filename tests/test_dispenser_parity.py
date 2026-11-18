"""Regression for the dispenser harness's numeric comparison.

`numbers_close` compares a vanilla "spread" token (`~<bound>`) with the Rust
value token by token.  An IEEE NaN bound used to make `abs(nan - value) > 0.4`
False, so the function fell through and reported a match for *every* Rust
value -- a false green.  These tests pin that a non-finite spread bound or
value fails closed while ordinary finite comparisons keep working.
"""
import unittest

from tools import test_dispenser_parity as dispenser


class NumbersCloseTest(unittest.TestCase):
    def test_nan_spread_bound_never_matches(self):
        # The oracle emitted "~nan"; it must not accept an unrelated value.
        self.assertFalse(dispenser.numbers_close('~nan 1 2', '12345 1 2'))

    def test_non_finite_bound_or_value_fails_closed(self):
        self.assertFalse(dispenser.numbers_close('~inf 1 2', '1.0 1 2'))
        self.assertFalse(dispenser.numbers_close('~-inf 1 2', '1.0 1 2'))
        self.assertFalse(dispenser.numbers_close('~1.0 1 2', 'nan 1 2'))
        self.assertFalse(dispenser.numbers_close('~1.0 1 2', 'inf 1 2'))

    def test_unparseable_bound_is_a_mismatch(self):
        self.assertFalse(dispenser.numbers_close('~abc 1 2', '1.0 1 2'))

    def test_finite_spread_behaviour_is_unchanged(self):
        self.assertTrue(dispenser.numbers_close('~1.0 1 2', '1.2 1 2'))
        self.assertFalse(dispenser.numbers_close('~1.0 1 2', '1.5 1 2'))
        # Plain tokens are still compared for exact equality.
        self.assertTrue(dispenser.numbers_close('1 2 3', '1 2 3'))
        self.assertFalse(dispenser.numbers_close('1 2 3', '1 9 3'))
        # A different token count still fails.
        self.assertFalse(dispenser.numbers_close('~1.0 1 2', '1.0 2'))


if __name__ == '__main__':
    unittest.main()
