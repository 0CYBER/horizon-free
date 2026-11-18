"""Pure-Python regression for the recipe book render harness's record comparison."""
import unittest

from tools import test_recipe_book_render_parity as harness


class SameTests(unittest.TestCase):
    def test_equal_poses_within_tolerance_are_equal(self):
        a = {'m': [1.0, 2.0, 3.0, 4.0], 'x': 7}
        b = {'m': [1.0009, 1.9998, 3.0, 4.0005], 'x': 7}
        self.assertTrue(harness.same(a, b))

    def test_equal_length_unequal_poses_are_not_equal(self):
        a = {'m': [1.0, 2.0, 3.0], 'x': 7}
        b = {'m': [1.0, 2.0, 9.0], 'x': 7}
        self.assertFalse(harness.same(a, b))

    def test_truncated_pose_tail_is_not_equal(self):
        # zip() alone drops the missing tail; the length check must catch it.
        vanilla = {'m': [1.0, 2.0, 3.0, 4.0], 'x': 7}
        native = {'m': [1.0, 2.0], 'x': 7}
        self.assertFalse(harness.same(vanilla, native))
        self.assertFalse(harness.same(native, vanilla))

    def test_truncated_list_field_is_not_equal(self):
        # Every list field, not just 'm', must agree in length.
        a = {'m': [], 'ids': [1, 2, 3]}
        b = {'m': [], 'ids': [1, 2]}
        self.assertFalse(harness.same(a, b))

    def test_none_and_missing_fields_are_not_equal(self):
        self.assertFalse(harness.same(None, {'m': [], 'x': 7}))
        self.assertFalse(harness.same({'m': [], 'x': 7}, {'m': []}))


if __name__ == '__main__':
    unittest.main()
