"""Tests for the layering scanner's Rust source helpers."""

import unittest

from tools.layering import rust_text as rt


class TestRegionsTest(unittest.TestCase):
    @staticmethod
    def regions(source):
        return rt.test_regions(rt.blank(source))

    def test_existing_test_attributes_still_work(self):
        for attr in ("#[test]", "#[cfg(test)]"):
            with self.subTest(attr=attr):
                source = f"{attr}\nfn t() {{}}\n"
                regions, mods = self.regions(source)
                self.assertEqual(len(regions), 1)
                self.assertTrue(rt.in_regions(regions, source.index("fn t")))
                self.assertEqual(mods, [])

    def test_stacked_cfg_all_test_is_a_test_region(self):
        source = '#[cfg(all(test, feature = "x"))]\nmod t {\n    fn helper() {}\n}\n'
        regions, _ = self.regions(source)
        self.assertEqual(len(regions), 1)
        self.assertTrue(rt.in_regions(regions, source.index("mod t")))
        self.assertTrue(rt.in_regions(regions, source.index("helper")))

    def test_other_cfg_shapes_that_require_test_are_test_regions(self):
        for attr in (
            "#[cfg(all(unix, test))]",
            '#[cfg(all(not(unix), test, feature = "x"))]',
            '#[cfg(any(test, all(test, feature = "x")))]',
            "#[cfg(not(not(test)))]",
            "#[cfg(not(any(not(test), unix)))]",
        ):
            with self.subTest(attr=attr):
                source = f"{attr}\nfn t() {{}}\n"
                regions, _ = self.regions(source)
                self.assertEqual(len(regions), 1)
                self.assertTrue(rt.in_regions(regions, source.index("fn t")))

    def test_production_enabled_cfgs_do_not_exempt_items_or_child_modules(self):
        for attr in (
            '#[cfg(any(test, feature = "x"))]',
            '#[cfg(all(feature = "x", any(unix, test)))]',
            '#[cfg(any(feature = "test", test))]',
            "#[cfg(not(all(test, unix)))]",
            "#[cfg_attr(test, inline)]",
            "#[cfg(all())]",
        ):
            for item in ("fn prod() {}", "mod child;"):
                with self.subTest(attr=attr, item=item):
                    regions, mods = self.regions(f"{attr}\n{item}\n")
                    self.assertEqual(regions, [])
                    self.assertEqual(mods, [])

    def test_negated_and_string_test_are_not_test_regions(self):
        for attr in (
            "#[cfg(not(test))]",
            "#[cfg(all(unix, not(test)))]",
            '#[cfg(feature = "test")]',
            "#[cfg(unix)]",
        ):
            with self.subTest(attr=attr):
                source = f"{attr}\nfn prod() {{}}\n"
                regions, _ = self.regions(source)
                self.assertEqual(regions, [])

    def test_boolean_cfg_literals_follow_rust_semantics(self):
        for attr, exempt in (
            ("#[cfg(any(test, false))]", True),
            ("#[cfg(any(test, true))]", False),
            ("#[cfg(not(true))]", True),
            ("#[cfg(not(false))]", False),
        ):
            for item in ("fn edge() { mod_api::run(); }", "mod child;"):
                with self.subTest(attr=attr, item=item):
                    source = f"{attr}\n{item}\n"
                    regions, mods = self.regions(source)
                    self.assertEqual(rt.in_regions(regions, source.index(item)), exempt)
                    self.assertEqual(mods, ["child"] if exempt and item == "mod child;" else [])

    def test_mod_declared_under_stacked_cfg_is_a_test_mod(self):
        source = '#[cfg(all(test, feature = "x"))]\nmod hidden;\n'
        _, mods = self.regions(source)
        self.assertEqual(mods, ["hidden"])


if __name__ == "__main__":
    unittest.main()
