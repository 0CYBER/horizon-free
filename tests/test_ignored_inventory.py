"""Descriptive ignores cannot vanish between normal and exact-selected daily tests."""
import tempfile
import unittest
from pathlib import Path

from tools.ci import ignored_inventory, ignored_tests


class IgnoredInventory(unittest.TestCase):
    def test_unselected_descriptive_ignore_and_flattened_daily_selection(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)

            def put(path, text):
                file = root / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(text)

            put("Cargo.toml", '[workspace]\nmembers = ["crates/horizon-client", "crates/horizon-relay"]\n')
            put("launcher/Cargo.toml", '[package]\nname = "horizon-launcher"\n[workspace]\nmembers = []\n')
            put("crates/horizon-client/Cargo.toml", '[package]\nname = "horizon-client"\n')
            put("crates/horizon-client/src/lib.rs", "mod game;\n")
            put("crates/horizon-client/src/game.rs", 'include!(concat!(env!("OUT_DIR"), "/game_modules.rs"));\n')
            put("crates/horizon-client/src/game/blocks/.game-area", "")
            put("crates/horizon-client/src/game/blocks/widget.rs",
                'mod tests { include!("widget_parts/tests.rs"); }\n')
            put("crates/horizon-client/src/game/blocks/widget_parts/tests.rs",
                '#[test] #[ignore = "daily"] fn daily_test() { assert_eq!(1, 1); }\n')
            put("crates/horizon-relay/Cargo.toml",
                '[package]\nname = "horizon-relay"\n[lib]\npath = "../relay-shared/lib.rs"\n')
            put("crates/relay-shared/lib.rs",
                'const EXAMPLE: &str = r#"#[test] #[ignore = "example"] fn not_a_test() {}"#;\n'
                '/* #[test] #[ignore = "comment"] fn also_not_a_test() {} */\n'
                'mod tests { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ignored_case.rs")); }\n')
            put("crates/horizon-relay/src/ignored_case.rs",
                '#[test] #[ignore = "needs prepared resources"] fn otherwise_unselected() {}\n')
            functions = ignored_inventory.tests(root)
            self.assertEqual(len(functions), 2)
            daily = ignored_tests.Row("horizon-client", "game::widget::tests::daily_test", "lib", "daily",
                                      "daily-horizon-client.txt", "resources")
            failures = ignored_inventory.problems([daily], functions, {})
            self.assertEqual(len(failures), 1)
            self.assertIn("horizon-relay tests::otherwise_unselected", failures[0])
            wrong = daily._replace(test="game::blocks::widget::tests::daily_test")
            self.assertEqual(len(ignored_inventory.problems([wrong], functions, {})), 3)
            allowed = {("horizon-relay", "lib", "tests::otherwise_unselected", "needs prepared resources"): {
                "rationale": "An external runner supplies this private fixture."}}
            self.assertEqual(ignored_inventory.problems([daily], functions, allowed), [])
            changed = [function._replace(reason="new descriptive ignore")
                       if function.package == "horizon-relay" else function for function in functions]
            self.assertEqual(len(ignored_inventory.problems([daily], changed, allowed)), 1)
            empty = [function._replace(reason="") if function.package == "horizon-relay" else function
                     for function in functions]
            self.assertEqual(len(ignored_inventory.problems([daily], empty, allowed)), 1)


if __name__ == "__main__":
    unittest.main()
