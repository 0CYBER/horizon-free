"""Regression: source-identity reviews must pin physical Git objects.

A git replace/graft ref must not let substituted bytes satisfy (or invalidate) a
record that names a full commit ID. No real refs are touched: every repository
here is created inside a TemporaryDirectory owned by this test.
"""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import check_security_review as review


class ReplacementRefTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        # Git treats the *presence* of GIT_NO_REPLACE_OBJECTS as disabling
        # replacements, even when its value is "0" (or "1"). An inherited value
        # would mask the baseline failure and let the fixed test pass vacuously,
        # so make the caller environment hermetic before building the fixture.
        environment = mock.patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        os.environ.pop("GIT_NO_REPLACE_OBJECTS", None)
        self.root = Path(temporary.name)
        self.env = dict(
            os.environ,
            GIT_AUTHOR_NAME="fixture",
            GIT_AUTHOR_EMAIL="fixture@example.invalid",
            GIT_COMMITTER_NAME="fixture",
            GIT_COMMITTER_EMAIL="fixture@example.invalid",
        )
        self.git("init", "-q", "-b", "main")
        self.git("config", "core.hooksPath", os.devnull)
        self.git("config", "commit.gpgSign", "false")

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.root, env=self.env,
            check=True, capture_output=True, text=True,
        ).stdout

    def commit_source(self, data, message):
        (self.root / "source.txt").write_bytes(data)
        self.git("add", "source.txt")
        # --allow-empty: a later fixture may reuse identical bytes on purpose.
        self.git("commit", "--allow-empty", "-qm", message)
        return self.git("rev-parse", "HEAD").strip()

    def record(self, commit, digest):
        return {
            "schema_version": 1,
            "source_commit": commit,
            "reviewed_at": "2026-11-16",
            "reviewer": {
                "identity": "Synthetic fixture",
                "method": "static_source_inspection",
                "independent_human_review": False,
            },
            "files": [{
                "path": "source.txt",
                "git_mode": "100644",
                "sha256": digest,
                "scope": [{"lines": [1, 1], "question": "Synthetic question"}],
                "result": "no_findings_in_reviewed_scope",
                "conclusion": "Only a fixture",
                "whole_file_status": "pending",
                "remaining": "The rest is unreviewed",
            }],
        }

    def test_replacement_ref_cannot_substitute_recorded_source_identity(self):
        original = b"original physical bytes\n"
        substituted = b"substituted bytes\n"
        orig_commit = self.commit_source(original, "original")
        repl_commit = self.commit_source(substituted, "replacement")
        self.git("replace", orig_commit, repl_commit)

        # Outcome check on the fixture itself, before review runs: in this
        # hermetic environment replacements must be active, so reading the
        # original commit's blob must yield the substituted bytes. This keeps
        # the unfixed code from passing vacuously under an inherited env1.
        self.assertEqual(self.git("show", f"{orig_commit}:source.txt"), substituted.decode())

        original_record = self.record(orig_commit, hashlib.sha256(original).hexdigest())
        substituted_record = self.record(orig_commit, hashlib.sha256(substituted).hexdigest())

        # A record over the physical bytes the commit actually names still verifies.
        self.assertEqual(review.verify(original_record, self.root), 1)
        # Substituted bytes must not satisfy a record naming the original commit.
        with self.assertRaisesRegex(ValueError, "SHA-256 differs"):
            review.verify(substituted_record, self.root)

    def test_against_comparison_uses_physical_bytes_under_replacement(self):
        physical = b"shared physical bytes\n"
        substituted = b"substituted against bytes\n"
        source_commit = self.commit_source(physical, "source")
        against_commit = self.commit_source(physical, "against (same bytes)")
        against_replacement = self.commit_source(substituted, "against replacement")
        self.git("replace", against_commit, against_replacement)

        # Outcome check: replacements are active in the fixture, so the against
        # commit's blob must read as the substituted bytes before review runs.
        self.assertEqual(self.git("show", f"{against_commit}:source.txt"), substituted.decode())

        record = self.record(source_commit, hashlib.sha256(physical).hexdigest())
        # Both commits hold the same physical bytes. A replacement on the
        # --against commit must not turn this into a false "changed" result.
        self.assertEqual(review.verify(record, self.root, against=against_commit), 1)


if __name__ == "__main__":
    unittest.main()
