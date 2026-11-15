"""tools/ci/check_game_boot_tests.py: an unignored test may not boot the game (tools/ci/rust_tests.py reads the source)."""
import contextlib
import io
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.ci import check_game_boot_tests as check, rust_tests


class RustTests(unittest.TestCase):
    def test_comments_and_literals_are_blanked_with_their_lines_kept(self):
        source = textwrap.dedent('''\
            let a = "fn fake() { with_game(); }"; // fn comment() {}
            /* fn block() { /* nested */ } */ let b = r#"fn raw() {}"#;
            let c = '{'; let d: &'static str = b"}"; let e = '\\'';
            ''')
        text = rust_tests.blank(source)
        self.assertEqual(text.count("\n"), source.count("\n"))
        self.assertEqual(len(text), len(source))
        self.assertNotIn("fn", text)
        self.assertNotIn("{", text.split("\n")[2])
        self.assertIn("'static", text)

    def test_functions_their_attributes_and_bodies(self):
        source = textwrap.dedent("""\
            #[cfg(test)]
            mod tests {
                #[test]
                #[ignore = "daily"]
                fn ignored() -> Result<(), Box<dyn Fn() -> u8>> { inner() }

                #[tokio::test]
                pub(crate) async fn runtime() { let x = "}"; }

                #[cfg_attr(miri, ignore)]
                #[test]
                fn conditionally_ignored() {}

                fn helper<F: Fn() -> u8>(f: F) where F: Copy { f(); }
                fn declared();
            }
            """)
        found = {function.name: function for function in rust_tests.functions(source)}
        self.assertEqual(set(found), {"ignored", "runtime", "conditionally_ignored", "helper"})
        self.assertEqual(found["ignored"].line, 5)
        self.assertTrue(found["ignored"].is_test and found["ignored"].is_ignored)
        self.assertIn("inner()", found["ignored"].body)
        self.assertTrue(found["runtime"].is_test and not found["runtime"].is_ignored)
        # Conditionally ignored still runs somewhere: not ignored, conservatively.
        self.assertFalse(found["conditionally_ignored"].is_ignored)
        self.assertFalse(found["ignored"].is_daily)
        kept = {function.name: function for function in rust_tests.functions(source, rust_tests.DAILY_KEEP)}
        self.assertTrue(kept["ignored"].is_daily and not kept["runtime"].is_daily)
        self.assertFalse(found["helper"].is_test)
        self.assertIn("f();", found["helper"].body)


class GameBootCheck(unittest.TestCase):
    NAMES = ["Game::new", "with_game"]

    def offenders(self, source):
        return [(function.name, chain) for function, chain in check.offenders(textwrap.dedent(source), self.NAMES)]

    def test_a_test_that_boots_directly_or_through_a_same_file_helper_is_named(self):
        self.assertEqual(self.offenders("""\
            fn server_game() -> Game { Game::new(1) }
            fn silent_game() -> Game { let g = server_game(); g }
            #[test]
            fn direct() { with_game(|game| {}); }
            #[test]
            fn through_helpers() { let game = silent_game(); }
            #[test]
            #[ignore = "daily"]
            fn moved() { with_game(|game| {}); }
            #[test]
            fn unit() { let text = "with_game(x)"; // with_game(y)
            }
            #[test]
            fn other_new() { Vec::new(); without_game(); }
            """), [("direct", "with_game"), ("through_helpers", "silent_game -> server_game -> Game::new")])

    def test_a_file_without_a_fixture_call_is_not_read_further(self):
        with patch.object(check.rust_tests, "functions") as functions:
            self.assertEqual(check.offenders("#[test]\nfn plain() { Game::default(); }\n", self.NAMES), [])
        functions.assert_not_called()

    def test_the_fixture_list_reads_names_and_refuses_anything_else(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixtures.txt"
            path.write_text("# note\nwith_game  # the usual one\n\nGame::new\n")
            self.assertEqual(check.fixtures(path), ["with_game", "Game::new"])
            path.write_text("with_game(\n")
            with self.assertRaises(SystemExit):
                check.fixtures(path)

    def test_the_committed_list_names_fixtures_that_exist(self):
        for name in check.fixtures():
            function = name.rsplit("::", 1)[-1]
            found = subprocess.run(["git", "grep", "-q", "-E", rf"fn {function}\b", "--", "crates/*.rs"],
                                   cwd=check.ROOT)
            self.assertEqual(found.returncode, 0, f"{name} is not defined anywhere under crates/")

    def test_main_fails_naming_each_offender(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a.rs").write_text("#[test]\nfn boots() { with_game(|g| {}); }\n")
            (root / "b.rs").write_text("#[test]\nfn fine() {}\n")
            with patch.object(check, "ROOT", root), patch.object(check, "fixtures", return_value=self.NAMES), \
                    contextlib.redirect_stdout(io.StringIO()) as log:
                self.assertEqual(check.main(["a.rs", "b.rs"]), 1)
                self.assertEqual(check.main(["b.rs"]), 0)
        self.assertIn("::error::a.rs:2: test `boots` boots the game (with_game)", log.getvalue())


if __name__ == "__main__":
    unittest.main()
