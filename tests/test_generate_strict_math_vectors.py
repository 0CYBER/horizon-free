"""Regression for generate_strict_math_vectors.Cases storing out-of-range u64 bits."""

import unittest

from tools import generate_strict_math_vectors as g


class AroundBitsTests(unittest.TestCase):
    def test_around_zero_stores_only_valid_u64_bits(self):
        cases = g.Cases()
        cases.around(0.0, "zero")

        # Every stored bit value must be a valid unsigned 64-bit integer so
        # generate() can render it as `0x{value:016x}` without a stray minus.
        for (value,) in cases.rows:
            self.assertGreaterEqual(value, 0)
            self.assertLessEqual(value, g.MASK)

        # The valid neighbours are unchanged by the range guard.
        self.assertEqual(set(cases.rows), {(0,), (1,), (2,)})

    def test_add_skips_invalid_u64_bits(self):
        cases = g.Cases()
        cases.add(-2)
        cases.add(-1)
        cases.add(g.MASK + 1)
        cases.add(0, label="zero")
        cases.add(g.MASK, label="max")

        self.assertEqual(set(cases.rows), {(0,), (g.MASK,)})


if __name__ == "__main__":
    unittest.main()
