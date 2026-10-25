import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
ZERO_OID = "0" * 40


@unittest.skipUnless(shutil.which("sh") and shutil.which("python3"), "Git hook needs sh and python3")
class PrePushTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        self.env = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
        self.git("init", "-b", "main")
        (self.repo / "tools").mkdir()
        shutil.copyfile(ROOT / "tools/check_public_tree.py", self.repo / "tools/check_public_tree.py")
        (self.repo / ".gitignore").write_text("/.local/\n")
        (self.repo / "README.md").write_text("Public fixture\n")
        self.git("add", ".gitignore", "README.md")
        self.git("commit", "-m", "Public root")
        self.base = self.git("rev-parse", "HEAD")

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", f"core.hooksPath={self.repo / 'no-hooks'}", "-c", "user.name=Test",
             "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false",
             "-c", "tag.gpgsign=false", *args],
            cwd=self.repo, env=self.env, text=True, stderr=subprocess.PIPE).strip()

    def commit_file(self, path, content="Public fixture\n"):
        destination = self.repo / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content)
        self.git("add", "-f", path)
        self.git("commit", "-m", f"Add {path}")
        return self.git("rev-parse", "HEAD")

    def private_commit(self):
        return self.commit_file(".local/artifact.txt", "Private fixture, not game data\n")

    def update(self, local_ref, local_oid, remote_ref=None, remote_oid=ZERO_OID):
        return f"{local_ref} {local_oid} {remote_ref or local_ref} {remote_oid}\n"

    def push(self, *updates):
        return subprocess.run(["sh", str(ROOT / ".githooks/pre-push"), "origin", "unused"],
                              cwd=self.repo, env=self.env, input="".join(updates),
                              text=True, capture_output=True)

    def assert_passed(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Public tree check passed.", result.stdout)

    def assert_private_rejected(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(".local/artifact.txt", result.stderr)

    def test_clean_branch_ignores_unpushed_refs_and_head(self):
        self.git("checkout", "-b", "local-only")
        private = self.private_commit()
        for ref in ["refs/remotes/archive/private", "refs/original/refs/heads/main", "refs/tags/private"]:
            self.git("update-ref", ref, private)
        self.git("branch", "clean", self.base)
        for remote_oid in [ZERO_OID, private]:
            with self.subTest(remote_oid=remote_oid):
                self.assert_passed(self.push(self.update("refs/heads/clean", self.base, remote_oid=remote_oid)))
        # The standalone audit still checks every local ref.
        audit = subprocess.run([sys.executable, "tools/check_public_tree.py", "--history"],
                               cwd=self.repo, env=self.env, text=True, capture_output=True)
        self.assert_private_rejected(audit)

    def test_raw_revision_push_uses_supplied_oid(self):
        self.private_commit()
        self.assert_passed(self.push(self.update(self.base, self.base, "refs/heads/public")))

    def test_unpushed_replacement_does_not_taint_clean_history(self):
        private = self.private_commit()
        tree = self.git("rev-parse", f"{private}^{{tree}}")
        replacement = self.git("commit-tree", tree, "-m", "Private replacement root")
        self.git("replace", self.base, replacement)
        self.assert_passed(self.push(self.update(self.base, self.base, "refs/heads/public")))

    def test_replacement_cannot_hide_private_history(self):
        private = self.private_commit()
        self.git("replace", private, self.base)
        self.assert_private_rejected(self.push(self.update("refs/heads/main", private)))

    def test_all_pushed_refs_are_checked(self):
        private = self.private_commit()
        clean_update = self.update("refs/heads/clean", self.base)
        private_update = self.update("refs/heads/main", private)
        for updates in [(clean_update, private_update), (private_update, clean_update)]:
            with self.subTest(updates=updates):
                self.assert_private_rejected(self.push(*updates))

    def test_private_ancestors_are_rejected_even_after_removal_and_an_earlier_push(self):
        self.private_commit()
        self.git("rm", ".local/artifact.txt")
        self.git("commit", "-m", "Remove private fixture")
        remote = self.git("rev-parse", "HEAD")
        tip = self.commit_file("public.txt")
        for remote_oid in [ZERO_OID, remote]:
            with self.subTest(remote_oid=remote_oid):
                self.assert_private_rejected(self.push(self.update("refs/heads/main", tip, remote_oid=remote_oid)))

    def test_annotated_tags_check_their_target_history(self):
        self.private_commit()
        self.git("rm", ".local/artifact.txt")
        self.git("commit", "-m", "Remove private fixture")
        self.git("tag", "-a", "clean", self.base, "-m", "Clean tag")
        self.git("tag", "-a", "private", "-m", "Private history tag")
        clean = self.git("rev-parse", "refs/tags/clean")
        private = self.git("rev-parse", "refs/tags/private")
        self.assert_passed(self.push(self.update("refs/tags/clean", clean)))
        self.assert_private_rejected(self.push(self.update("refs/tags/private", private)))

    def test_empty_push_and_deletions_do_not_scan_unpushed_history(self):
        private = self.private_commit()
        deletion = self.update("(delete)", ZERO_OID, "refs/heads/private", private)
        clean_update = self.update(self.base, self.base, "refs/heads/public")
        for updates in [(), (deletion,), (deletion, clean_update)]:
            with self.subTest(updates=updates):
                self.assert_passed(self.push(*updates))

    def test_private_file_introduced_only_by_merge_resolution_is_rejected(self):
        self.git("checkout", "-b", "feature")
        self.commit_file("feature.txt")
        self.git("checkout", "main")
        self.commit_file("main.txt")
        self.git("merge", "--no-ff", "--no-commit", "feature")
        merge = self.private_commit()
        self.assert_private_rejected(self.push(self.update("refs/heads/main", merge)))

    def test_invalid_pushed_oid_fails_closed(self):
        result = self.push(self.update("refs/heads/missing", "f" * 40))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("Public tree check passed.", result.stdout)


if __name__ == "__main__":
    unittest.main()
