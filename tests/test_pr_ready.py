import argparse
import unittest
from unittest import mock

from tools import pr_ready

PACKAGES = {"horizon-core": "crates/horizon-core", "horizon-client": "crates/horizon-client",
            "horizon-theme": "crates/horizon-theme", "xtask": "xtask"}


def args(host="local", quick=False):
    return argparse.Namespace(host=host, quick=quick)


class PrReadyTests(unittest.TestCase):
    def test_paths_map_to_the_crates_that_own_them(self):
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core/src/jmath.rs", "crates/horizon-client/build.rs",
                                                  "xtask/src/main.rs", "tools/parity.py"}, PACKAGES),
                         ["horizon-client", "horizon-core", "xtask"])
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core/README.md"}, PACKAGES), [])
        # The generated stylesheet is checked by horizon-theme's tests.
        self.assertEqual(pr_ready.touched_crates({"website/src/tokens.css"}, PACKAGES), ["horizon-theme"])
        # A crate whose name prefixes another's directory doesn't own it.
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core-extra/x.rs"}, PACKAGES), [])
        # The workspace manifest and lock touch every crate.
        self.assertIsNone(pr_ready.touched_crates({"Cargo.lock", "crates/horizon-core/src/lib.rs"}, PACKAGES))

    def test_which_workspace_builds_a_path(self):
        self.assertTrue(pr_ready.workspace_rust("crates/horizon-theme/fonts/x.ttf"))
        self.assertTrue(pr_ready.workspace_rust("Cargo.toml"))
        self.assertFalse(pr_ready.workspace_rust("crates/horizon-client/README.md"))
        self.assertFalse(pr_ready.workspace_rust("launcher/src/main.rs"))
        self.assertTrue(pr_ready.launcher_rust("launcher/src/main.rs"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-prepare/src/lib.rs"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-theme/src/lib.rs"))
        self.assertFalse(pr_ready.launcher_rust("launcher/links.json"))
        self.assertFalse(pr_ready.launcher_rust("launcher/README.md"))
        self.assertFalse(pr_ready.launcher_rust("crates/horizon-client/src/game.rs"))

    def test_scope_warns_on_size_and_mixed_areas(self):
        self.assertEqual(pr_ready.scope_warnings({"crates/horizon-client/src/game.rs", "tools/test_food_parity.py",
                                                  "mods/horizon/src/test/java/dev/horizonlab/parity/X.java",
                                                  "docs/native-client.md"}, 120), [])
        self.assertEqual(pr_ready.scope_warnings({"crates/horizon-theme/src/lib.rs", "launcher/src/theme.rs"}, 10), [])
        warnings = pr_ready.scope_warnings({"crates/horizon-client/src/game.rs", "launcher/src/main.rs",
                                            ".github/workflows/ci.yml"}, 900)
        self.assertEqual(len(warnings), 2)
        self.assertIn("900 lines of code: over 500", warnings[0])
        self.assertEqual(warnings[1], "mixes CI, game, launcher")

    def test_documentation_and_tests_are_not_counted_in_the_size(self):
        self.assertTrue(pr_ready.reviewed("tools/pr_ready.py"))
        self.assertTrue(pr_ready.reviewed("crates/horizon-client/src/game.rs"))
        self.assertFalse(pr_ready.reviewed("CONTRIBUTING.md"))
        self.assertFalse(pr_ready.reviewed("tests/test_pr_ready.py"))
        self.assertFalse(pr_ready.reviewed("tools/test_food_parity.py"))
        self.assertFalse(pr_ready.reviewed("crates/horizon-core/tests/jmath.rs"))

    def test_cargo_lock_changes_only_with_a_manifest(self):
        self.assertIsNone(pr_ready.lock_problem({"crates/horizon-core/src/lib.rs"}))
        self.assertIsNone(pr_ready.lock_problem({"Cargo.lock", "crates/horizon-client/Cargo.toml"}))
        self.assertIsNone(pr_ready.lock_problem({"launcher/Cargo.lock", "crates/horizon-theme/Cargo.toml"}))
        self.assertIn("Cargo.lock", pr_ready.lock_problem({"Cargo.lock", "crates/horizon-core/src/lib.rs"}))

    def test_label_follows_what_players_get(self):
        self.assertEqual(pr_ready.suggested_label({"tools/pr_ready.py", "CONTRIBUTING.md"}), "`skip-release`")
        self.assertEqual(pr_ready.suggested_label({"crates/horizon-world/examples/probe.rs"}), "`skip-release`")
        self.assertIn("feature", pr_ready.suggested_label({"crates/horizon-client/src/game.rs"}))
        self.assertIn("feature", pr_ready.suggested_label({"launcher/src/main.rs"}))

    def test_affected_harnesses_come_from_the_parity_runner(self):
        found = {"food": "tools/test_food_parity.py", "datafix#1": "tools/test_datafix_parity.py",
                 "datafix#2": "tools/test_datafix_parity.py"}
        with mock.patch.object(pr_ready.parity, "harnesses", return_value=found), \
                mock.patch.object(pr_ready.parity, "load_state", return_value={"harnesses": {
                    "food": {"dependencies": ["crates/horizon-world/src/food.rs"]},
                    "datafix#1": {"dependencies": ["crates/horizon-world/src/datafix.rs"]},
                    "datafix#2": {"dependencies": ["crates/horizon-world/src/datafix.rs"]}}}), \
                mock.patch.object(pr_ready.parity, "ROOT", pr_ready.parity.Path(".")):
            self.assertEqual(pr_ready.affected_here({"crates/horizon-world/src/datafix.rs"}), ["datafix"])
            self.assertEqual(pr_ready.affected_here({"README.md"}), [])
            self.assertEqual(pr_ready.affected_here({"Cargo.lock"}), ["food", "datafix"])

    def test_harnesses_are_listed_when_they_cannot_run(self):
        changed = {"crates/horizon-world/src/food.rs"}
        with mock.patch.object(pr_ready, "affected_here", return_value=["food", "light"]):
            steps = pr_ready.plan(args(quick=True), "origin/main", changed, {"horizon-world": "crates/horizon-world"})
        harness = steps[-1]
        self.assertEqual(harness.status, "list")
        self.assertIn("2 affected, to run in CI (--quick): food, light", harness.detail)
        by_name = {step.name: step for step in steps}
        self.assertIn("tests: horizon-world", by_name)
        self.assertEqual(by_name["launcher clippy"].status, "skip")
        self.assertIsNone(by_name["clippy"].status)
        with mock.patch.object(pr_ready, "affected_here", return_value=["food"]):
            steps = pr_ready.plan(args(host="macmini"), "origin/main", changed, {"horizon-world": "crates/horizon-world"})
        self.assertIn("--changed-files", steps[-1].command)
        self.assertIn("crates/horizon-world/src/food.rs", steps[-1].command)

    def test_summaries_of_check_output(self):
        output = "test result: ok. 3 passed; 0 failed;\ntest result: FAILED. 4 passed; 1 failed;\n"
        self.assertEqual(pr_ready.tests_passed(output), "7 passed, 1 failed")
        self.assertEqual(pr_ready.unittest_passed("....\nRan 12 tests in 1.0s\n\nOK\n"), "12 passed")
        self.assertEqual(pr_ready.unittest_passed("Ran 12 tests in 1.0s\n\nFAILED (failures=2)\n"),
                         "12 run, FAILED (failures=2)")
        parity_output = ("3 files changed: 2 harnesses depend on them, 2 without a pass for their current inputs.\n"
                         "\n1 passed, 1 failed in 2.0 min.\n  light: /x/.local/parity-runner/logs/light.log\n")
        self.assertEqual(pr_ready.parity_summary(parity_output), "2 affected, 1 passed, failed: light")
        self.assertEqual(pr_ready.parity_summary("2 files changed: 0 harnesses depend on them, 0 without a pass for "
                                                 "their current inputs.\nNothing to run: every harness passed."),
                         "no harness affected")
        self.assertEqual(pr_ready.duration(125), "2 min 5 s")


if __name__ == "__main__":
    unittest.main()
