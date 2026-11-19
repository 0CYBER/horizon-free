"""Regression: HUD parity's float tolerance applies at every nesting depth."""
import unittest

from tools import test_hud_parity as hud


class RecursiveCloseTests(unittest.TestCase):
    def test_nested_dict_in_list_accepts_float_rounding(self):
        vanilla = {'runs': [{'scale': 0.1 + 0.2}]}
        horizon = {'runs': [{'scale': 0.3}]}
        self.assertTrue(hud.close(vanilla, horizon))
        self.assertTrue(hud.same(vanilla, horizon))

    def test_deeply_mixed_containers_accept_float_rounding(self):
        vanilla = {'draws': [{'matrix': [1.0, 0.1 + 0.2, {'t': 0.1 + 0.2}]}, {'type': 'text'}]}
        horizon = {'draws': [{'matrix': [1.0, 0.3, {'t': 0.3}]}, {'type': 'text'}]}
        self.assertTrue(hud.same(vanilla, horizon))

    def test_non_numeric_and_structure_still_compare_exactly(self):
        self.assertFalse(hud.same({'runs': [{'type': 'sprite'}]}, {'runs': [{'type': 'text'}]}))
        self.assertFalse(hud.same({'runs': [{'scale': 0.3}]}, {'runs': [{'scale': 0.3}, {'scale': 0.4}]}))
        self.assertFalse(hud.same({'runs': [{'scale': 0.3}]}, {'runs': {'scale': 0.3}}))
        self.assertFalse(hud.same({'flag': True}, {'flag': False}))
        self.assertFalse(hud.same({'name': 'a'}, {'name': 'b'}))


if __name__ == '__main__':
    unittest.main()
