import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tools import check_security_review as review


class ScopedReviewTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = dict(os.environ, GIT_AUTHOR_NAME="fixture", GIT_AUTHOR_EMAIL="fixture@example.invalid",
                        GIT_COMMITTER_NAME="fixture", GIT_COMMITTER_EMAIL="fixture@example.invalid")
        self.git("init", "-q", "-b", "main")
        self.git("config", "core.hooksPath", "/dev/null" if os.name != "nt" else "NUL")
        self.git("config", "commit.gpgSign", "false")
        self.data = b"fn reviewed() {}\nfn pending() {}\n"
        (self.root / "source.rs").write_bytes(self.data)
        self.git("add", "source.rs")
        self.git("commit", "-qm", "fixture")
        self.commit = self.git("rev-parse", "HEAD").strip()
        self.record = {
            "schema_version": 1, "source_commit": self.commit, "reviewed_at": "2026-11-12",
            "reviewer": {"identity": "Synthetic fixture", "method": "static_source_inspection",
                         "independent_human_review": False},
            "files": [{"path": "source.rs", "git_mode": "100644", "sha256": hashlib.sha256(self.data).hexdigest(),
                       "scope": [{"lines": [1, 1], "question": "Synthetic question"}],
                       "result": "no_findings_in_reviewed_scope", "conclusion": "Only a fixture",
                       "whole_file_status": "pending", "remaining": "The second function is unreviewed"}],
        }

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, env=self.env, check=True,
                              capture_output=True, text=True).stdout

    def test_exact_source_matches_without_approving_other_code(self):
        before = copy.deepcopy(self.record)
        self.assertEqual(review.verify(self.record, self.root, against="HEAD"), 1)
        self.assertEqual(self.record, before)
        self.assertEqual(self.record["files"][0]["whole_file_status"], "pending")

    def test_wrong_hash_is_rejected(self):
        self.record["files"][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "SHA-256 differs"):
            review.verify(self.record, self.root)

    def test_cli_requires_canonical_full_commit_ids_in_both_object_formats(self):
        for object_format in ("sha1", "sha256"):
            root = self.root / object_format
            root.mkdir()

            def git(*args):
                return subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull,
                                       "-c", "commit.gpgSign=false", *args], cwd=root, env=self.env,
                                      check=True, capture_output=True, text=True).stdout.strip()

            git("init", "-q", "-b", "main", "--object-format=" + object_format)
            (root / "source.rs").write_bytes(self.data)
            git("add", "source.rs")
            git("commit", "-qm", "identity fixture")
            commit = git("rev-parse", "HEAD")
            # Run the real CLI from this fixture checkout, without executing reviewed code.
            checker = root / "tools/check_security_review.py"
            checker.parent.mkdir()
            shutil.copyfile(review.__file__, checker)
            record = copy.deepcopy(self.record)
            candidates = [(commit, 0), (commit[:12], 1)]
            if object_format == "sha256":
                candidates.append((commit[:40], 1))
            else:
                moving_ref = "a" * 64
                git("branch", moving_ref)
                candidates.append((moving_ref, 1))
            for candidate, expected in candidates:
                with self.subTest(object_format=object_format, source_commit=candidate):
                    record["source_commit"] = candidate
                    record_path = root / "record.json"
                    record_path.write_text(json.dumps(record), encoding="utf-8")
                    result = subprocess.run([sys.executable, str(checker), str(record_path)],
                                            cwd=root, env=self.env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                    if expected:
                        self.assertIn("full lowercase Git commit ID", result.stderr)

    def test_new_revision_does_not_inherit_review(self):
        (self.root / "source.rs").write_bytes(self.data + b"fn new_code() {}\n")
        self.git("add", "source.rs")
        self.git("commit", "-qm", "changed fixture")
        self.assertEqual(review.verify(self.record, self.root), 1)
        with self.assertRaisesRegex(ValueError, "changed at HEAD"):
            review.verify(self.record, self.root, against="HEAD")

    def test_invalid_scope_does_not_count_as_review(self):
        for lines in ([], [0, 1], [2, 1], [1, 3], [True, 1]):
            with self.subTest(lines=lines):
                self.record["files"][0]["scope"][0]["lines"] = lines
                with self.assertRaisesRegex(ValueError, "scope line range"):
                    review.verify(self.record, self.root)

    def test_mode_change_also_requires_review(self):
        self.git("update-index", "--chmod=+x", "source.rs")
        self.git("commit", "-qm", "mode fixture")
        with self.assertRaisesRegex(ValueError, "changed at HEAD"):
            review.verify(self.record, self.root, against="HEAD")

    def test_unsafe_paths_and_duplicate_entries_are_rejected(self):
        for path in ("../source.rs", "/source.rs", "./source.rs", "a//b", "a\\b"):
            with self.subTest(path=path):
                record = copy.deepcopy(self.record)
                record["files"][0]["path"] = path
                with self.assertRaisesRegex(ValueError, "repository-relative"):
                    review.verify(record, self.root)
        self.record["files"].append(copy.deepcopy(self.record["files"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            review.verify(self.record, self.root)

    def test_hash_cannot_be_used_as_whole_file_approval(self):
        self.record["files"][0]["whole_file_status"] = "safe"
        with self.assertRaisesRegex(ValueError, "cannot approve"):
            review.verify(self.record, self.root)

    def test_symlink_is_not_a_reviewed_regular_file(self):
        self.git("update-index", "--add", "--cacheinfo", "120000", self.git("hash-object", "source.rs").strip(), "link")
        self.git("commit", "-qm", "symlink fixture")
        self.record["source_commit"] = self.git("rev-parse", "HEAD").strip()
        self.record["files"][0]["path"] = "link"
        with self.assertRaisesRegex(ValueError, "not a symlink"):
            review.verify(self.record, self.root)
