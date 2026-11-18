"""Newly introduced data-fixer-table blobs must be rejected in publication ranges and history.

Fixtures use invented `minecraft:synthetic.N` ids: the retired tables' fingerprint shape, with no
Mojang bytes. Each case is an isolated Git repository so main's config, refs and history stay untouched.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from tools import check_public_tree


SCRIPT = Path(__file__).resolve().parents[1] / "tools" / "check_public_tree.py"
ENV = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
       "PYTHONDONTWRITEBYTECODE": "1"}


def table_text(entries):
    return "\n".join(f'("minecraft:synthetic.{n}", "minecraft:synthetic_{n}"),' for n in range(entries)) + "\n"


class PublicTableHistoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
             "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
            cwd=self.repo, env=ENV, text=True, stderr=subprocess.PIPE).strip()

    def check(self, *args, input=None):
        return subprocess.run([sys.executable, str(SCRIPT), *args], cwd=self.repo, env=ENV,
                              input=input, capture_output=True, text=True)

    def init_repo(self, *flags):
        result = subprocess.run(["git", "init", "-q", *flags, "-b", "main"], cwd=self.repo, env=ENV,
                                capture_output=True)
        if result.returncode != 0:
            self.skipTest(f"git init {' '.join(flags)} unsupported: "
                          + result.stderr.decode("utf-8", "replace").strip())

    def write(self, path, text):
        destination = self.repo / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text)

    def commit(self, path, text):
        self.write(path, text)
        self.git("add", "-f", path)
        self.git("commit", "-q", "-m", f"add {path}")
        return self.git("rev-parse", "HEAD")

    def test_add_then_delete_table_is_rejected_by_range_and_history(self):
        self.git("init", "-q", "-b", "main")
        base = self.commit("README.md", "public fixture\n")
        table = table_text(60)
        added = self.commit("fixtures/new_table.json", table)
        self.assertEqual(self.git("show", f"{added}:fixtures/new_table.json"), table.strip())

        index = self.check()
        self.assertEqual(index.returncode, 1, index.stderr)
        self.assertIn("fixtures/new_table.json", index.stderr)

        self.git("rm", "-q", "fixtures/new_table.json")
        self.git("commit", "-q", "-m", "delete table")

        clean = self.check()
        self.assertEqual(clean.returncode, 0, clean.stderr)

        for mode in (["--range", f"{base}..HEAD"], ["--history"]):
            with self.subTest(mode=mode):
                result = self.check(*mode)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("fixtures/new_table.json", result.stderr)

    def test_merge_only_table_then_delete_is_rejected_by_publication_modes(self):
        self.git("init", "-q", "-b", "main")
        base = self.commit("README.md", "public fixture\n")
        self.git("switch", "-q", "-c", "side")
        self.commit("side.txt", "side\n")
        self.git("switch", "-q", "main")
        self.commit("main.txt", "main\n")
        self.git("merge", "--no-ff", "--no-commit", "side")
        # Neither parent has the table: only the merge's tree introduces it.
        self.commit("fixtures/merge_table.json", table_text(60))
        self.git("rm", "-q", "fixtures/merge_table.json")
        self.git("commit", "-q", "-m", "delete merge table")
        clean = self.check()
        self.assertEqual(clean.returncode, 0, clean.stderr)

        head = self.git("rev-parse", "HEAD")
        push = f"refs/heads/main {head} refs/heads/main {base}\n"
        for mode, input in ((["--range", f"{base}..HEAD"], None),
                            (["--history"], None), (["--pre-push"], push)):
            with self.subTest(mode=mode):
                result = self.check(*mode, input=input)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("fixtures/merge_table.json", result.stderr)

    def test_legacy_baseline_blob_and_legal_files_are_not_rejected(self):
        self.git("init", "-q", "-b", "main")
        legacy = table_text(60)
        self.write("src/legacy_table.rs", legacy)
        self.write("src/small_table.rs", table_text(5))
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "base with legacy table")
        base = self.git("rev-parse", "HEAD")
        legacy_blob = self.git("rev-parse", f"{base}:src/legacy_table.rs")

        # The tree-only check still rejects the legacy table while it is tracked at base.
        at_base = self.check()
        self.assertEqual(at_base.returncode, 1, at_base.stderr)
        self.assertIn("src/legacy_table.rs", at_base.stderr)

        self.git("mv", "src/legacy_table.rs", "src/legacy_table_renamed.rs")
        self.git("commit", "-q", "-m", "rename legacy table")
        self.assertEqual(self.git("rev-parse", "HEAD:src/legacy_table_renamed.rs"), legacy_blob)
        self.commit("docs/note.md", "ordinary documentation, no table data\n")
        self.git("rm", "-q", "src/legacy_table_renamed.rs")
        self.git("commit", "-q", "-m", "retire legacy table")

        # A rename carries the baseline blob to a new path; it is not newly introduced.
        for mode in (["--range", f"{base}..HEAD"], ["--history"]):
            with self.subTest(mode=mode):
                result = self.check(*mode)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Public tree check passed.", result.stdout)
        self.assertEqual(self.check().returncode, 0)

    def test_gitlink_changes_are_ignored_but_a_replacement_blob_is_scanned(self):
        self.git("init", "-q", "-b", "main")
        base = self.commit("README.md", "public fixture\n")
        path = "vendor/submodule"
        # A gitlink names a commit, which need not exist in this repository's object database.
        for commit in (base, "f" * 40):
            self.git("update-index", "--add", "--cacheinfo", f"160000,{commit},{path}")
            self.git("commit", "-q", "-m", "change gitlink")
            head = self.git("rev-parse", "HEAD")
            push = f"refs/heads/main {head} refs/heads/main {base}\n"
            for mode, input in ((["--range", f"{base}..HEAD"], None),
                                (["--history"], None), (["--pre-push"], push)):
                with self.subTest(commit=commit, mode=mode):
                    result = self.check(*mode, input=input)
                    self.assertEqual(result.returncode, 0, result.stderr)

        # A type change back to a file must still scan the destination blob at the same path.
        self.commit(path, table_text(60))
        self.git("rm", "-q", path)
        self.git("commit", "-q", "-m", "delete replacement table")
        head = self.git("rev-parse", "HEAD")
        push = f"refs/heads/main {head} refs/heads/main {base}\n"
        for mode, input in ((["--range", f"{base}..HEAD"], None),
                            (["--history"], None), (["--pre-push"], push)):
            with self.subTest(mode=mode):
                result = self.check(*mode, input=input)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(path, result.stderr)

    def test_staged_table_is_ignored_in_history_and_empty_push(self):
        self.git("init", "-q", "-b", "main")
        base = self.commit("README.md", "public fixture\n")
        self.commit("docs/note.md", "ordinary documentation\n")
        # Stage a table in the index but never commit it. A history scan reads committed objects and
        # a deletion-only push publishes none, so neither may judge this unrelated staged content.
        self.write("fixtures/staged_table.json", table_text(60))
        self.git("add", "-f", "fixtures/staged_table.json")

        plain = self.check()
        self.assertEqual(plain.returncode, 1, plain.stderr)
        self.assertIn("fixtures/staged_table.json", plain.stderr)

        for mode in (["--history"], ["--range", f"{base}..HEAD"]):
            with self.subTest(mode=mode):
                result = self.check(*mode)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Public tree check passed.", result.stdout)

        head = self.git("rev-parse", "HEAD")
        empty_push = f"refs/heads/main {'0' * 40} refs/heads/main {head}\n"
        pushed = self.check("--pre-push", input=empty_push)
        self.assertEqual(pushed.returncode, 0, pushed.stderr)
        self.assertIn("Public tree check passed.", pushed.stdout)

    def test_mixed_fingerprints_below_threshold_are_allowed(self):
        self.git("init", "-q", "-b", "main")
        base = self.commit("README.md", "public fixture\n")
        # 25 matches of each fingerprint: copied_tables() requires TABLE_ENTRIES per pattern (OR),
        # not a total split across both, so 25 + 25 is not a copy of the retired tables.
        rows = [f'("minecraft:synthetic.{n}", "minecraft:synthetic_{n}"),' for n in range(25)]
        rows += [f"{n}\tminecraft:synthetic\t\tminecraft:synthetic\tkind={n}" for n in range(25)]
        mixed = "\n".join(rows) + "\n"
        self.commit("fixtures/mixed.json", mixed)
        self.git("rm", "-q", "fixtures/mixed.json")
        self.git("commit", "-q", "-m", "delete mixed")
        for mode in (["--range", f"{base}..HEAD"], ["--history"]):
            with self.subTest(mode=mode):
                result = self.check(*mode)
                self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.check().returncode, 0)

    def test_sha256_baseline_blob_and_rename_are_allowed(self):
        self.init_repo("--object-format=sha256")
        legacy = table_text(60)
        self.write("src/legacy_table.rs", legacy)
        self.write("src/small_table.rs", table_text(5))
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "base with legacy table")
        base = self.git("rev-parse", "HEAD")
        legacy_blob = self.git("rev-parse", f"{base}:src/legacy_table.rs")
        self.assertEqual(len(legacy_blob), 64)

        self.git("mv", "src/legacy_table.rs", "src/legacy_table_renamed.rs")
        self.git("commit", "-q", "-m", "rename legacy table")
        self.assertEqual(self.git("rev-parse", "HEAD:src/legacy_table_renamed.rs"), legacy_blob)
        self.git("rm", "-q", "src/legacy_table_renamed.rs")
        self.git("commit", "-q", "-m", "retire legacy table")

        # Baseline blobs have 64-char ids in a SHA-256 repo; a truncated candidate id would not
        # compare equal and the grandfathered blob would wrongly look newly introduced.
        for mode in (["--range", f"{base}..HEAD"], ["--history"]):
            with self.subTest(mode=mode):
                result = self.check(*mode)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Public tree check passed.", result.stdout)

    def test_sha256_new_table_blob_is_rejected(self):
        self.init_repo("--object-format=sha256")
        base = self.commit("README.md", "public fixture\n")
        added = self.commit("fixtures/new_table.json", table_text(60))
        self.assertEqual(len(added), 64)
        self.git("rm", "-q", "fixtures/new_table.json")
        self.git("commit", "-q", "-m", "delete table")
        for mode in (["--range", f"{base}..HEAD"], ["--history"]):
            with self.subTest(mode=mode):
                result = self.check(*mode)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("fixtures/new_table.json", result.stderr)

    def test_cat_file_failure_fails_closed(self):
        self.git("init", "-q", "-b", "main")
        self.commit("README.md", "public fixture\n")
        previous = Path.cwd()
        os.chdir(self.repo)
        self.addCleanup(os.chdir, previous)
        with self.assertRaisesRegex(RuntimeError, "missing blob"):
            check_public_tree.table_blobs(["0" * 40])


if __name__ == "__main__":
    unittest.main()
