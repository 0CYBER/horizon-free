#!/usr/bin/env python3
"""Verify source identity and scope metadata, never certify code as safe.

    python3 tools/check_security_review.py docs/security-reviews/*.json
    python3 tools/check_security_review.py docs/security-reviews/*.json --against HEAD
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def fields(value, expected, where):
    if not isinstance(value, dict) or set(value) != set(expected.split()):
        raise ValueError(f"{where}: expected fields {expected}")


def text(value, where):
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{where}: expected nonempty text")


def git(root, *arguments):
    env = dict(os.environ, GIT_NO_LAZY_FETCH="1", GIT_TERMINAL_PROMPT="0")
    result = subprocess.run(["git", "--literal-pathspecs", *arguments], cwd=root, env=env,
                            capture_output=True)
    if result.returncode:
        raise ValueError("Git object unavailable; fetch the recorded commit or check the path")
    return result.stdout


def source(root, commit, path):
    listing = git(root, "ls-tree", "-z", commit, "--", path).split(b"\0")
    if len(listing) != 2 or not listing[0]:
        raise ValueError(f"{path}: expected one tracked regular file")
    metadata, _, name = listing[0].partition(b"\t")
    mode, kind, _ = metadata.split()
    if name.decode("utf-8") != path or kind != b"blob" or mode not in (b"100644", b"100755"):
        raise ValueError(f"{path}: expected one tracked regular file, not a symlink or directory")
    return mode.decode("ascii"), git(root, "cat-file", "blob", f"{commit}:{path}")


def verify(document, root=ROOT, against=None):
    fields(document, "schema_version source_commit reviewed_at reviewer files", "record")
    if type(document["schema_version"]) is not int or document["schema_version"] != 1:
        raise ValueError("Unsupported schema_version")
    commit = document["source_commit"]
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit):
        raise ValueError("source_commit must be a full lowercase Git commit ID")
    canonical = git(root, "rev-parse", "--verify", "--end-of-options", f"{commit}^{{commit}}").strip().decode("ascii")
    if canonical != commit:
        raise ValueError("source_commit must be a full lowercase Git commit ID, not an abbreviation or ref")
    date = document["reviewed_at"]
    if not isinstance(date, str) or datetime.date.fromisoformat(date).isoformat() != date:
        raise ValueError("reviewed_at must be an ISO date")
    reviewer = document["reviewer"]
    fields(reviewer, "identity method independent_human_review", "reviewer")
    text(reviewer["identity"], "reviewer.identity")
    text(reviewer["method"], "reviewer.method")
    if type(reviewer["independent_human_review"]) is not bool:
        raise ValueError("independent_human_review must be a boolean, not an inferred approval")
    entries = document["files"]
    if not isinstance(entries, list) or not entries:
        raise ValueError("files must be a nonempty list")
    seen = set()
    for entry in entries:
        fields(entry, "path git_mode sha256 scope result conclusion whole_file_status remaining", "file")
        path = entry["path"]
        text(path, "path")
        parts = PurePosixPath(path).parts
        if (PurePosixPath(path).is_absolute() or any(p in (".", "..") for p in parts)
                or PurePosixPath(path).as_posix() != path or any(c in path for c in "\\\0\n\r:")):
            raise ValueError("path must be a normalized repository-relative POSIX path")
        if path in seen:
            raise ValueError(f"{path}: duplicate review entry")
        seen.add(path)
        if entry["whole_file_status"] != "pending":
            raise ValueError(f"{path}: this scoped format cannot approve a whole file")
        if entry["result"] not in ("no_findings_in_reviewed_scope", "observations"):
            raise ValueError(f"{path}: unsupported scoped result")
        text(entry["conclusion"], "conclusion")
        text(entry["remaining"], "remaining")
        digest = entry["sha256"]
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError(f"{path}: expected a lowercase SHA-256 digest")
        mode, data = source(root, commit, path)
        if mode != entry["git_mode"] or hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f"{path}: mode or SHA-256 differs from the recorded commit")
        line_count = len(data.decode("utf-8").splitlines())
        if not isinstance(entry["scope"], list) or not entry["scope"]:
            raise ValueError(f"{path}: scope must contain explicit line ranges and questions")
        for scope in entry["scope"]:
            fields(scope, "lines question", "scope")
            lines = scope["lines"]
            if (not isinstance(lines, list) or len(lines) != 2 or any(type(n) is not int for n in lines)
                    or not 1 <= lines[0] <= lines[1] <= line_count):
                raise ValueError(f"{path}: invalid scope line range")
            text(scope["question"], "scope.question")
        if against is not None:
            target = git(root, "rev-parse", "--verify", "--end-of-options", f"{against}^{{commit}}").strip().decode()
            current_mode, current_data = source(root, target, path)
            if current_mode != mode or current_data != data:
                raise ValueError(f"{path}: changed at {against}; review the new bytes before carrying conclusions forward")
    return len(entries)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("records", type=Path, nargs="+")
    parser.add_argument("--against", help="also reject changed source bytes or modes at this Git revision")
    args = parser.parse_args(argv)
    try:
        for record in args.records:
            count = verify(json.loads(record.read_text(encoding="utf-8")), against=args.against)
            print(f"{record}: {count} scoped record(s) match source; whole-file security remains pending")
    except (OSError, ValueError, TypeError) as error:
        print(f"security review: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
