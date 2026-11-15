"""tools/ci/move_to_daily.py: which tests move to the daily run, and how they are marked and listed."""
import contextlib
import io
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.ci import check_game_boot_tests, move_to_daily

SOURCE = textwrap.dedent("""\
    fn helper() {}

    #[cfg(test)]
    mod tests {
        /// Docs.
        #[test]
        fn slow_one() {
            helper();
        }

        #[tokio::test(flavor = "multi_thread")]
        #[allow(unused)]
        async fn async_one() {}

        #[test] fn same_line() {}

        #[test]
        #[ignore = "needs the fixture"]
        fn ignored_one() {}

        macro_rules! cases { ($name:ident) => { #[test] fn $name() {} }; }
        cases!(generated);
    }
    """)


class MoveToDaily(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.policy = self.root / "ignored-tests"
        self.policy.mkdir()
        (self.policy / "flaky.txt").write_text("pkg tests::listed #1 flaky\n")
        source = self.root / "crates/pkg/src/lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text(SOURCE)
        self.source = source

    def line(self, name):
        return next(number for number, text in enumerate(SOURCE.splitlines(), 1) if f"fn {name}" in text)

    def row(self, name, line=None, reason="slow", target="lib"):
        return ("pkg", target, f"tests::{name}", f"crates/pkg/src/lib.rs:{line or self.line(name)}", reason)

    def apply(self, rows):
        return move_to_daily.apply(rows, root=self.root, folder=self.policy)

    def test_marks_after_the_test_attribute_and_lists_sorted_rows(self):
        moved, already, skipped = self.apply([self.row("slow_one"), self.row("async_one", reason="game-boot")])
        self.assertEqual((len(moved), already, skipped), (2, [], []))
        text = self.source.read_text()
        self.assertIn('    #[test]\n    #[ignore = "daily"]\n    fn slow_one() {', text)
        self.assertIn('    #[tokio::test(flavor = "multi_thread")]\n    #[ignore = "daily"]\n    #[allow(unused)]\n'
                      "    async fn async_one", text)
        rows = [line for line in (self.policy / "daily-pkg.txt").read_text().splitlines() if not line.startswith("#")]
        self.assertEqual(rows, ["pkg tests::async_one lib daily game-boot", "pkg tests::slow_one lib daily slow"])

    def test_restore_undoes_apply(self):
        rows = [self.row("slow_one"), self.row("async_one")]
        self.apply(rows)
        restored, skipped = move_to_daily.restore(rows[:1], root=self.root, folder=self.policy)
        self.assertEqual((len(restored), skipped), (1, []))
        text = self.source.read_text()
        self.assertIn("    #[test]\n    fn slow_one() {", text)
        self.assertIn('#[ignore = "daily"]\n    #[allow(unused)]\n    async fn async_one', text)
        kept = [line for line in (self.policy / "daily-pkg.txt").read_text().splitlines() if not line.startswith("#")]
        self.assertEqual(kept, ["pkg tests::async_one lib daily slow"])
        restored, skipped = move_to_daily.restore([self.row("slow_one")], root=self.root, folder=self.policy)
        self.assertEqual(restored, [])
        self.assertEqual(skipped[0][1], "no daily mark above the fn")

    def test_running_again_changes_nothing(self):
        rows = [self.row("slow_one"), self.row("async_one")]
        self.apply(rows)
        text, listed = self.source.read_text(), (self.policy / "daily-pkg.txt").read_text()
        moved, already, skipped = self.apply(rows)
        self.assertEqual((moved, len(already), skipped), ([], 2, []))
        self.assertEqual((self.source.read_text(), (self.policy / "daily-pkg.txt").read_text()), (text, listed))

    def test_a_marked_test_without_its_row_gets_the_row(self):
        self.apply([self.row("slow_one")])
        (self.policy / "daily-pkg.txt").unlink()
        moved, _, _ = self.apply([self.row("slow_one")])
        self.assertEqual(len(moved), 1)
        self.assertEqual(self.source.read_text().count('#[ignore = "daily"]'), 1)
        self.assertIn("pkg tests::slow_one lib daily slow", (self.policy / "daily-pkg.txt").read_text())

    def test_what_it_cannot_place_is_reported_and_left_alone(self):
        rows = [
            self.row("same_line"),
            self.row("ignored_one"),
            self.row("generated", line=self.line("$name")),
            self.row("missing", line=3),
            ("pkg", "lib", "tests::listed", f"crates/pkg/src/lib.rs:{self.line('slow_one')}", "slow"),
        ]
        moved, already, skipped = self.apply(rows)
        self.assertEqual((moved, already), ([], []))
        reasons = {row[2]: why for row, why in skipped}
        self.assertIn("on a line of its own", reasons["tests::same_line"])
        self.assertIn("another reason: refused", reasons["tests::ignored_one"])
        self.assertIn("macro-generated", reasons["tests::generated"])
        self.assertIn("is not `fn missing`", reasons["tests::missing"])
        self.assertIn("already listed in flaky.txt", reasons["tests::listed"])
        self.assertEqual(self.source.read_text(), SOURCE)
        self.assertFalse((self.policy / "daily-pkg.txt").exists())

    def test_a_moved_line_is_found_when_the_name_is_the_files_only_one(self):
        moved, _, skipped = self.apply([self.row("slow_one", line=1)])
        self.assertEqual((len(moved), skipped), (1, []))

    def test_a_file_in_two_targets_gets_one_mark_and_a_row_per_target(self):
        moved, _, _ = self.apply([self.row("slow_one"), ("pkg", "test:copy", "slow_one",
                                                         f"crates/pkg/src/lib.rs:{self.line('slow_one')}", "slow")])
        self.assertEqual(len(moved), 2)
        self.assertEqual(self.source.read_text().count('#[ignore = "daily"]'), 1)
        listed = (self.policy / "daily-pkg.txt").read_text()
        self.assertIn("pkg slow_one test:copy daily slow", listed)
        self.assertIn("pkg tests::slow_one lib daily slow", listed)

    def test_rows_of_another_target_name_it(self):
        self.apply([self.row("slow_one", target="test:integration")])
        self.assertIn("pkg tests::slow_one test:integration daily slow", (self.policy / "daily-pkg.txt").read_text())

    def test_select_moves_slow_tests_and_game_boot_tests_only(self):
        source = self.root / "crates/pkg/src/game.rs"
        source.write_text(textwrap.dedent("""\
            fn boot() { Game::new(); }
            #[test]
            fn boots() { boot(); }
            #[test]
            fn plain() {}
            """))
        measurement = self.root / "verify-tests.tsv"
        header = "package\ttarget_kind\ttarget\ttest\tsource\tmin_seconds\tmax_seconds\n"
        measurement.write_text(header + "".join(f"{package}\t{kind}\t{target}\t{test}\t{where}\t0\t{seconds}\n" for (
            package, kind, target, test, where, seconds) in [
            ("pkg", "lib", "pkg", "boots", "crates/pkg/src/game.rs:3", "0.01"),
            ("pkg", "lib", "pkg", "plain", "crates/pkg/src/game.rs:5", "1.99"),
            ("pkg", "test", "integration", "slow", "crates/pkg/tests/integration.rs:4", "2.01"),
            ("pkg", "bin", "pkg-tool", "slow_bin", "crates/pkg/src/main.rs:4", "9"),
            ("pkg", "doctest", "pkg", "src/lib.rs - f (line 3)", "crates/pkg/src/lib.rs:3", "4"),
        ]))
        with patch.object(check_game_boot_tests, "fixtures", return_value=["Game::new"]):
            rows, skipped = move_to_daily.select(measurement, root=self.root)
        self.assertEqual(rows, [
            ("pkg", "bin:pkg-tool", "slow_bin", "crates/pkg/src/main.rs:4", "slow"),
            ("pkg", "lib", "boots", "crates/pkg/src/game.rs:3", "game-boot"),
            ("pkg", "test:integration", "slow", "crates/pkg/tests/integration.rs:4", "slow"),
        ])
        self.assertEqual(len(skipped), 1)
        self.assertIn("not a Rust #[test]", skipped[0])

    def test_the_command_reads_a_list_and_rejects_a_malformed_one(self):
        listing = self.root / "list.tsv"
        listing.write_text("# comment\n" + "\t".join(self.row("slow_one")) + "\n")
        self.assertEqual(move_to_daily.read_list(listing), [self.row("slow_one")])
        listing.write_text("pkg\tlib\ttests::slow_one\n")
        with self.assertRaises(SystemExit), contextlib.redirect_stdout(io.StringIO()):
            move_to_daily.read_list(listing)


if __name__ == "__main__":
    unittest.main()
