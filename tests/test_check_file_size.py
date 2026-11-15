import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tools import check_file_size, merge_base


class roots:
    """Patches the repository root of the checker and of the shared merge-base helpers together."""

    def __init__(self, root):
        self.patches = [mock.patch.object(check_file_size, "ROOT", root), mock.patch.object(merge_base, "ROOT", root)]

    def start(self):
        for patch in self.patches:
            patch.start()

    def stop(self):
        for patch in self.patches:
            patch.stop()

    def __enter__(self):
        self.start()

    def __exit__(self, *exc):
        self.stop()


def entry(path, before, now, generated=False):
    return (path, before, now, generated)


class ProblemsTests(unittest.TestCase):
    def test_a_big_file_may_not_grow(self):
        found = check_file_size.problems([entry("crates/a/src/game.rs", 21000, 21001)])
        self.assertEqual(len(found), 1)
        self.assertIn("docs/layers.md#file-size", found[0])

    def test_daily_marks_do_not_grow_a_big_file_but_code_still_does(self):
        before = 'fn helper() {}\n' * 3001 + '#[test]\nfn daily_test() {}\n'
        marked = before.replace('#[test]\n', '#[test]\n    #[ignore = "daily"]\n')
        count = check_file_size.code_lines
        self.assertEqual(check_file_size.problems([entry("tests.rs", count(before), count(marked))]), [])
        self.assertEqual(count(before), count(marked))
        self.assertEqual(len(check_file_size.problems([
            entry("tests.rs", count(marked), count(marked + 'fn added() {}\n'))])), 1)

    def test_a_big_file_may_shrink_or_stay(self):
        self.assertEqual(check_file_size.problems([entry("a.rs", 21000, 20000), entry("b.rs", 5000, 5000)]), [])

    def test_small_files_grow_up_to_the_limit(self):
        self.assertEqual(check_file_size.problems([entry("a.rs", 2000, 3000)]), [])
        self.assertEqual(len(check_file_size.problems([entry("a.rs", 2000, 3001)])), 1)

    def test_a_new_big_file_fails(self):
        self.assertIn("a new file", check_file_size.problems([entry("a.rs", 0, 3500)])[0])

    def test_the_message_names_both_ways_out(self):
        found = check_file_size.problems([entry("crates/a/src/mobs/ai.rs", 3100, 3101)])[0]
        self.assertIn('ai_tests.rs (`#[path = "ai_tests.rs"] mod tests;`)', found)
        self.assertIn("pure-move PR first", found)

    def test_third_party_and_generated_files_are_not_checked(self):
        self.assertEqual(check_file_size.problems([entry("third_party/naga/src/x.rs", 8000, 9000),
                                                   entry("a.rs", 4000, 4100, generated=True)]), [])

    def test_a_test_filename_does_not_exempt_non_inline_code(self):
        for path in ("feature_tests.rs", "crates/a/src/mod_content_tests.rs",
                     "crates/b/src/level/tests.rs", "tests.rs"):
            self.assertEqual(len(check_file_size.problems([entry(path, 3100, 3101)])), 1, path)
        self.assertEqual(len(check_file_size.problems([entry("tests.rs", 0, 3500)])), 1)


TEST_MODULE = """#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braces_in_literals_and_comments() {
        let text = "}\\" {";
        let close = '}';
        let open = '{';
        let quote = '\\'';
        let raw = r#"
}
{"#;
        // }
        /* } /* nested { */ } */
        assert!(text.contains(close) && open != quote && !raw.is_empty());
    }
}
"""


def skipped(text):
    return check_file_size.test_module_lines(text)


class TestModuleTests(unittest.TestCase):
    """Inline test modules don't count; anything else, and anything the rule isn't sure about, does."""

    def test_a_test_module_is_skipped_with_the_braces_in_its_literals(self):
        self.assertEqual(skipped("fn a() {}\n\n" + TEST_MODULE + "\nfn b() {}\n"), TEST_MODULE.count("\n"))

    def test_visibility_and_nesting(self):
        text = ("pub mod outer {\n    #[cfg(test)]\n    pub(crate) mod a {\n        fn f() {\n        }\n    }\n"
                "    fn real() {}\n}\n#[cfg(test)]\npub mod b {\n    #[cfg(test)]\n    mod c {\n    }\n}\n")
        self.assertEqual(skipped(text), 5 + 6)
        self.assertEqual(check_file_size.code_lines(text), 3)

    def test_windows_line_endings(self):
        self.assertEqual(skipped(TEST_MODULE.replace("\n", "\r\n")), TEST_MODULE.count("\n"))

    def test_other_test_items_count(self):
        for text in ("#[cfg(test)]\nfn helper() {\n}\n", "#[cfg(test)]\nmod tests;\n",
                     "#[cfg(test)]\nimpl A {\n}\n", "#[cfg(not(test))]\nmod live {\n}\n"):
            self.assertEqual(skipped(text), 0, text)

    def test_a_module_quoted_in_a_string_counts(self):
        quoted = "const SOURCE: &str = r#\"\n#[cfg(test)]\nmod tests {\n}\n\"#;\n"
        self.assertEqual(skipped(quoted), 0)
        self.assertEqual(skipped("/*\n#[cfg(test)]\nmod tests {\n}\n*/\n"), 0)

    def test_a_module_not_laid_out_by_rustfmt_counts(self):
        for text in ("#[cfg(test)]\nmod tests {\n    fn a() {}\n} // end\nfn real() {}\n}\n",
                     "#[cfg(test)]\nmod tests {\nfn a() {}\n}\n",
                     "#[cfg(test)]\nmod tests {\n    fn a() {}\n    }\nfn real() {\n}\n",
                     "#[cfg(test)]\nmod tests { fn a() {} }\nfn real() {\n}\n",
                     "#[cfg(test)]\nmod tests {\n    fn a() {}\n    } mod live { pub fn real() {}\n}\n",
                     "#[cfg(test)]\nmod tests {\n    fn a() {}\n"):
            self.assertEqual(skipped(text), 0, text)


class GitTests(unittest.TestCase):
    """Against a real repository: renames compare with the old name, the working tree counts."""

    def test_rename_and_growth_in_a_repository(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                       GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env)

            run("init", "-q", "-b", "main")
            (root / "big.rs").write_text("x\n" * 3100)
            # UTF-8 that Windows' code page cannot decode (0x9d): `git show` output is read as UTF-8.
            (root / "small.rs").write_bytes("// ”\n".encode() * 10)
            run("add", ".")
            run("commit", "-q", "-m", "base")
            run("checkout", "-q", "-b", "work")
            run("mv", "big.rs", "moved.rs")
            run("commit", "-q", "-m", "rename")
            (root / "small.rs").write_text("y\n" * 20)
            with roots(root), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(check_file_size.main(["--base", "main"]), 0)
                (root / "moved.rs").write_text("x\n" * 3101)
                self.assertEqual(check_file_size.main(["--base", "main"]), 1)

    def test_test_modules_dont_count_on_either_side(self):
        """The base (git show) and the working tree count the same way: a file over the limit may grow inside its test
        module, but not outside it."""
        def source(code, tests):
            return "x\n" * code + "#[cfg(test)]\nmod tests {\n" + "    t();\n" * tests + "}\n"

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                       GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env)

            run("init", "-q", "-b", "main")
            (root / "big.rs").write_text(source(3100, 200))
            run("add", ".")
            run("commit", "-q", "-m", "base")
            run("checkout", "-q", "-b", "work")
            out = io.StringIO()
            with roots(root), contextlib.redirect_stdout(out):
                (root / "big.rs").write_text(source(3100, 300))
                run("commit", "-q", "-am", "more tests")
                self.assertEqual(check_file_size.main(["--base", "main"]), 0)
                (root / "big.rs").write_text(source(3101, 100))
                self.assertEqual(check_file_size.main(["--base", "main"]), 1)
            self.assertIn("big.rs: 3101 lines without test modules (3100 on the base)", out.getvalue())


    def test_a_pull_request_merge_is_measured_against_main_as_merged(self):
        """main grew a file over the limit after the branch's base (a sync's growth): a pull request that doesn't
        touch it stays green, measured on its merge into that main; one that grows it further still fails."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                       GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env,
                                      encoding="utf-8").stdout.strip()

            def numbered(start, stop):
                return "".join(f"line {i}\n" for i in range(start, stop))

            run("init", "-q", "-b", "main")
            (root / "big.rs").write_text(numbered(0, 3100))
            (root / "small.rs").write_text("x\n")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            base = run("rev-parse", "HEAD")
            run("checkout", "-q", "-b", "untouched")
            (root / "small.rs").write_text("x\ny\n")
            run("commit", "-q", "-am", "small change")
            run("checkout", "-q", "-b", "grows", base)
            (root / "big.rs").write_text("line -1\n" + numbered(0, 3100))
            run("commit", "-q", "-am", "grow the big file at its top")
            run("checkout", "-q", "main")
            (root / "big.rs").write_text(numbered(0, 3200))
            run("commit", "-q", "-am", "main grows the big file at its end")
            out = io.StringIO()
            with roots(root), contextlib.redirect_stdout(out):
                for branch, expected in (("untouched", 0), ("grows", 1)):
                    run("checkout", "-q", "--detach", "main")
                    run("merge", "-q", "--no-ff", "-m", f"merge {branch}", branch)
                    with self.subTest(branch=branch):
                        self.assertEqual(check_file_size.main(["--base", base, "--pull-request-merge"]), expected)
                    if branch == "untouched":
                        # Against the branch's old base, main's own growth was charged to the pull request.
                        self.assertEqual(check_file_size.main(["--base", base]), 1)
            self.assertIn("big.rs: 3201 lines without test modules (3200 on the base)", out.getvalue())
            with roots(root), contextlib.redirect_stdout(io.StringIO()) as log:
                run("checkout", "-q", "grows")
                self.assertEqual(check_file_size.main(["--base", base, "--pull-request-merge"]), 1)
            self.assertIn("--pull-request-merge needs HEAD to merge the pull request", log.getvalue())



class SyncBaselineTests(unittest.TestCase):
    def test_the_cap_allows_equal_or_lower_but_not_growth(self):
        self.assertEqual(check_file_size.problems([entry("a.rs", 3100, 3200)], baseline={"a.rs": 3200}), [])
        self.assertEqual(check_file_size.problems([entry("a.rs", 3100, 3199)], baseline={"a.rs": 3200}), [])
        self.assertTrue(check_file_size.problems([entry("a.rs", 3100, 3201)], baseline={"a.rs": 3200}))

    def test_only_a_sync_declaration_allows_new_or_raised_entries(self):
        sizes, previous, counts = {"a.rs": 3200, "b.rs": 3300}, {"a.rs": 3100}, {"a.rs": 3200, "b.rs": 3300}
        self.assertEqual(len(check_file_size.baseline_problems(sizes, previous, counts, False)), 2)
        self.assertEqual(check_file_size.baseline_problems(sizes, previous, counts, True), [])
        self.assertEqual(check_file_size.baseline_problems({"a.rs": 3000}, previous, {"a.rs": 3000}, False), [])

    def test_stale_high_and_deleted_entries_fail_even_in_syncs(self):
        found = check_file_size.baseline_problems({"a.rs": 3200, "gone.rs": 3500}, {}, {"a.rs": 3199}, True)
        self.assertEqual(len(found), 2)
        self.assertTrue(all("stale-high" in problem for problem in found))

    def test_the_sync_marker_is_a_full_sha_and_entries_are_unique(self):
        marker = "a" * 40
        self.assertEqual(check_file_size.parse_baseline(f"# sync-baseline: {marker}\na.rs 3200\n"),
                         (marker, {"a.rs": 3200}))
        with self.assertRaises(ValueError):
            check_file_size.parse_baseline("# sync-baseline: not-a-sha")
        with self.assertRaises(ValueError):
            check_file_size.parse_baseline("a.rs 3200\na.rs 3300")


class SyncProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.run_git("init", "-q", "-b", "main")
        self.run_git("config", "user.name", "Fixture")
        self.run_git("config", "user.email", "fixture@example.invalid")
        self.commit("base")
        self.base = self.run_git("rev-parse", "HEAD").strip()
        self.run_git("checkout", "-q", "-b", "upstream")
        self.commit("upstream")
        self.upstream = self.run_git("rev-parse", "HEAD").strip()
        self.remote = self.root / "upstream.git"
        self.run_git("init", "-q", "--bare", str(self.remote))
        self.run_git("push", "-q", str(self.remote), "HEAD:refs/heads/main")
        environment = mock.patch.dict(os.environ, {"HORIZON_UPSTREAM_URL": str(self.remote)})
        environment.start()
        self.addCleanup(environment.stop)
        self.run_git("checkout", "-q", "-b", "work", self.base)
        self.commit("fork")
        patch = roots(self.root)
        patch.start()
        self.addCleanup(patch.stop)

    def run_git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, check=True,
                              capture_output=True, text=True).stdout

    def commit(self, name):
        (self.root / name).write_text(name)
        self.run_git("add", name)
        self.run_git("commit", "-q", "-m", name)

    def test_real_sync_parent_passes(self):
        self.run_git("merge", "-q", "--no-edit", "upstream")
        self.assertEqual(merge_base.sync_provenance(self.upstream, self.base), [])

    def test_fake_sha_missing_header_and_base_ancestor_fail(self):
        for marker in ["b" * 40, None, self.base]:
            with self.subTest(marker=marker):
                self.assertTrue(merge_base.sync_provenance(marker, self.base))

    def test_an_upstream_base_ancestor_that_is_a_merge_parent_is_rejected(self):
        # A forced merge makes the old base an exact merge parent, and it is
        # also in upstream main. Only the outside-base rule rejects this SHA.
        self.run_git("checkout", "-q", "-b", "already-based", self.base)
        self.run_git("merge", "-q", "--no-ff", "--no-edit", "upstream")
        result = merge_base.sync_provenance(self.base, self.base)
        self.assertIn("already an ancestor of the base", result[0])

    def test_ancestor_without_merge_parent_is_not_a_sync(self):
        self.run_git("checkout", "-q", "upstream")
        self.commit("child")
        self.assertTrue(merge_base.sync_provenance(self.upstream, self.base))

    def test_a_fork_commit_is_not_an_upstream_main_ancestor(self):
        fork = self.run_git("rev-parse", "HEAD").strip()
        self.assertIn("not an ancestor of upstream main",
                      merge_base.sync_provenance(fork, self.base)[0])

    def test_unreachable_upstream_fails_even_with_a_cached_ref(self):
        self.run_git("merge", "-q", "--no-edit", "upstream")
        self.assertEqual(merge_base.sync_provenance(self.upstream, self.base), [])
        with mock.patch.dict(os.environ, {"HORIZON_UPSTREAM_URL": str(self.root / "missing.git")}):
            result = merge_base.sync_provenance(self.upstream, self.base)
        self.assertIn("upstream main fetch failed", result[0])

    def test_fork_side_branch_merge_cannot_exempt_a_new_production_file(self):
        self.run_git("checkout", "-q", "-b", "forgery", self.base)
        self.commit("forged-parent")
        forged = self.run_git("rev-parse", "HEAD").strip()
        self.run_git("checkout", "-q", "work")
        self.run_git("merge", "-q", "--no-edit", "forgery")
        self.assertIn("not an ancestor of upstream main",
                      merge_base.sync_provenance(forged, self.base)[0])
        (self.root / "tools").mkdir()
        (self.root / "tools/file_size_baseline.txt").write_text(
            f"# sync-baseline: {forged}\nproduction.rs 3101\n")
        (self.root / "production.rs").write_text("fn production() {}\n" * 3101)
        self.run_git("add", "tools/file_size_baseline.txt", "production.rs")
        self.run_git("commit", "-q", "-m", "forge size exemption")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertNotEqual(check_file_size.main(["--base", self.base]), 0)
        self.assertIn("not an ancestor of upstream main", output.getvalue())
        self.assertIn("baseline added or raised", output.getvalue())
        checker = self.root / "tools/check_file_size.py"
        checker.write_text(Path(check_file_size.__file__).read_text())
        (self.root / "tools/merge_base.py").write_text(Path(merge_base.__file__).read_text())
        result = subprocess.run([sys.executable, str(checker), "--base", self.base],
                                cwd=self.root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("not an ancestor of upstream main", result.stdout)

    def test_shallow_history_is_an_explicit_failure(self):
        self.run_git("merge", "-q", "--no-edit", "upstream")
        with tempfile.TemporaryDirectory() as clone:
            subprocess.run(["git", "clone", "-q", "--depth", "1", self.root.as_uri(), clone], check=True)
            with roots(Path(clone)):
                result = merge_base.sync_provenance(self.upstream, self.base)
                self.assertIn("unshallow", result[0])

SYNC_A, SYNC_B = "a" * 40, "b" * 40


class RepositoryCase(unittest.TestCase):
    """A real repository whose base commit holds big.rs (3100 lines) and a sync baseline that lists it."""

    base_cap = 3200

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        (self.root / "tools").mkdir()
        self.write_baseline(SYNC_A, {"big.rs": self.base_cap})
        self.write("big.rs", 3100)
        self.git("add", ".")
        self.git("commit", "-q", "-m", "base")
        self.git("checkout", "-q", "-b", "work")
        patch = roots(self.root)
        patch.start()
        self.addCleanup(patch.stop)

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True, text=True).stdout

    def write(self, path, lines):
        (self.root / path).parent.mkdir(parents=True, exist_ok=True)
        (self.root / path).write_text("x\n" * lines)

    def write_baseline(self, marker, sizes):
        rows = [f"# sync-baseline: {marker}"] + [f"{path} {size}" for path, size in sizes.items()]
        (self.root / "tools/file_size_baseline.txt").write_text("\n".join(rows) + "\n")

    def commit(self, message="work"):
        self.git("add", ".")
        self.git("commit", "-q", "-m", message)

    def check(self, provenance=()):
        out = io.StringIO()
        with mock.patch.object(check_file_size, "sync_provenance", return_value=list(provenance)), \
                contextlib.redirect_stdout(out):
            return check_file_size.main(["--base", "main"]), out.getvalue()


class BaselineModeTests(RepositoryCase):
    """The checked-in caps are used by a sync PR (its header changed) and ignored by every other PR."""

    def test_shrinking_a_listed_file_needs_no_baseline_edit(self):
        # The cap on the base (3200) is above the file; outside a sync nothing forces it down.
        self.write("big.rs", 3050)
        self.commit()
        code, out = self.check()
        self.assertEqual(code, 0, out)
        self.assertIn("baselines not used", out)
        # Main can shrink independently after this branch forks. Its newer tip is not
        # this PR's baseline: 3050 still shrinks the common ancestor's 3100 lines.
        self.git("checkout", "-q", "main")
        self.write("big.rs", 3025)
        self.commit("independent main shrink")
        self.git("checkout", "-q", "work")
        code, out = self.check()
        self.assertEqual(code, 0, out)

    def test_a_listed_file_still_may_not_grow_outside_a_sync(self):
        # Its entry would allow 3200 lines, but the entry is not used: the base rule holds at 3100.
        self.write("big.rs", 3150)
        self.commit()
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("big.rs: 3150 lines without test modules (3100 on the base)", out)

    def test_raising_an_entry_without_a_sync_header_still_fails(self):
        self.write_baseline(SYNC_A, {"big.rs": 3300})
        self.commit()
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("added or raised without a changed sync-baseline upstream SHA", out)

    def test_a_sync_header_makes_the_entries_exact_caps(self):
        self.write_baseline(SYNC_B, {"big.rs": 3200})
        self.write("big.rs", 3200)
        self.commit()
        self.assertEqual(self.check()[0], 0)
        self.write("big.rs", 3201)
        self.commit("grow")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("3201 lines exceeds its sync baseline 3200", out)

    def test_a_sync_header_keeps_the_stale_high_rule(self):
        self.write_baseline(SYNC_B, {"big.rs": 3200})
        self.write("big.rs", 3150)
        self.commit()
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("big.rs: stale-high baseline 3200", out)

    def test_a_sync_header_needs_its_provenance(self):
        self.write_baseline(SYNC_B, {"big.rs": 3200})
        self.write("big.rs", 3200)
        self.commit()
        code, out = self.check(provenance=["sync baseline SHA is not an ancestor of upstream main"])
        self.assertEqual(code, 1)
        self.assertIn("not an ancestor of upstream main", out)


def load_generator():
    spec = importlib.util.spec_from_file_location(
        "write_file_size_baseline", Path(__file__).resolve().parent.parent / "tools/sync/write_file_size_baseline.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class GeneratorTests(RepositoryCase):
    """tools/sync/write_file_size_baseline.py writes what the checker accepts in a sync, and nothing else."""

    def setUp(self):
        super().setUp()
        self.generator = load_generator()

    def run_generator(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = self.generator.main(["--base", "main", *args])
        return code, out.getvalue(), err.getvalue()

    def baseline(self):
        return (self.root / "tools/file_size_baseline.txt").read_text()

    def test_it_lists_only_the_files_that_grew_and_the_checker_accepts_the_result(self):
        self.write("big.rs", 3150)           # over the limit and grew
        self.write("crossed.rs", 3200)       # was under the limit, now over it
        self.write("shrunk.rs", 100)
        self.write("third_party/x/lib.rs", 9000)
        (self.root / "gen.rs").write_text("// @generated\n" + "x\n" * 4000)
        self.commit()
        code, out, err = self.run_generator("--marker", SYNC_B)
        self.assertEqual((code, err), (0, ""), out)
        self.assertEqual(self.baseline().splitlines()[2:], [
            f"# sync-baseline: {SYNC_B}", "big.rs 3150", "crossed.rs 3200"])
        self.commit("regenerate")
        self.assertEqual(self.check()[0], 0)
        # Entries are exact: one more line fails, and a smaller file is stale-high until regenerated.
        self.write("big.rs", 3151)
        self.commit("grow")
        self.assertEqual(self.check()[0], 1)
        self.write("big.rs", 3100)
        self.commit("shrink")
        self.assertIn("stale-high", self.check()[1])
        self.assertEqual(self.run_generator("--marker", SYNC_B)[0], 0)
        self.commit("regenerate again")
        self.assertEqual(self.check()[0], 0)

    def test_growth_needs_a_new_upstream_marker(self):
        self.write("big.rs", 3150)
        self.commit()
        before = self.baseline()
        for args in ((), ("--marker", SYNC_A)):
            code, out, err = self.run_generator(*args)
            self.assertEqual(code, 2)
            self.assertIn("pass --marker", err)
            self.assertEqual(self.baseline(), before)

    def test_without_growth_it_writes_an_empty_ratchet_and_keeps_the_header(self):
        self.write("big.rs", 3000)
        self.commit()
        self.assertEqual(self.run_generator()[0], 0)
        self.assertEqual(self.baseline().splitlines()[2:], [f"# sync-baseline: {SYNC_A}"])
        self.commit("regenerate")
        self.assertEqual(self.check()[0], 0)

    def test_a_malformed_marker_is_refused(self):
        self.write("big.rs", 3150)
        self.commit()
        before = self.baseline()
        code, out, err = self.run_generator("--marker", "abc")
        self.assertEqual(code, 2)
        self.assertIn("Refusing to write", err)
        self.assertEqual(self.baseline(), before)

    def test_check_prints_and_leaves_the_file_alone(self):
        self.write("big.rs", 3150)
        self.commit()
        before = self.baseline()
        code, out, err = self.run_generator("--marker", SYNC_B, "--check")
        self.assertEqual(code, 0)
        self.assertIn("big.rs 3150", out)
        self.assertEqual(self.baseline(), before)



if __name__ == "__main__":
    unittest.main()
