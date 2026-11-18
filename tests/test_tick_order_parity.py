"""Pure-Python checks for the tick-order harness's compare(): text is feedback too, not just its digits."""
import unittest
from unittest import mock

from tools import test_tick_order_parity as tick


class CompareTests(unittest.TestCase):
    def test_changed_text_without_digits_is_off(self):
        # The regression: numbers() extracts nothing from either side, but 'alpha' != 'beta'.
        key = (0, 'mob', 'CustomName')
        one, other = {key: 'alpha'}, {key: 'beta'}
        self.assertEqual(tick.numbers('alpha'), [])
        self.assertEqual(tick.numbers('beta'), [])
        off, first, worst, total = tick.compare(one, other)
        self.assertEqual(off, 1)
        self.assertEqual(first, 0)
        self.assertEqual(worst, float('inf'))
        self.assertEqual(total, 1)

    def test_changed_non_numeric_content_next_to_numbers_is_off(self):
        # A unit or field name is feedback as well: the same number with a different suffix is still a difference.
        one = {(2, 'mob', 'Health'): 'Health: 10.0f'}
        other = {(2, 'mob', 'Health'): 'Health: 10.0d'}
        self.assertEqual(tick.compare(one, other)[0], 1)

    def test_numeric_tolerance_still_excuses_small_deltas_only(self):
        key = (0, 'mob', 'Pos')
        with mock.patch.object(tick, 'TOLERANCE', 1e-6):
            small = ({key: '1.0000000, 2.0000000, 3.0000000'},
                     {key: '1.00000005, 2.0000000, 3.0000000'})
            self.assertEqual(tick.compare(*small)[0], 0)
            big = ({key: '1.0, 2.0, 3.0'}, {key: '1.0, 2.5, 3.0'})
            off, first, worst, total = tick.compare(*big)
            self.assertEqual(off, 1)
            self.assertEqual(first, 0)
            self.assertEqual(worst, 0.5)
            self.assertEqual(total, 1)

    def test_missing_value_is_off_and_whitespace_is_normalized(self):
        key = (0, 'mob', 'CustomName')
        self.assertEqual(tick.compare({key: 'alpha'}, {}), (1, 0, float('inf'), 1))
        self.assertEqual(tick.compare({key: '  alpha  '}, {key: 'alpha'})[0], 0)


if __name__ == '__main__':
    unittest.main()
