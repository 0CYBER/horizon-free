#!/usr/bin/env python3
"""Reject captured-tool notices in tracked files and malformed workflow YAML.

Run from the repository root. Untracked and ignored files are deliberately outside
this check. No paths are exempt; literal examples in tests are assembled in pieces.
Uses the pinned, checksummed offline parser under tools/ci/vendor/.
"""

import argparse
import hashlib
import io
import os
import re
import subprocess
import sys
from pathlib import Path

PATTERNS = (
    r"Showing lines [0-9]+-[0-9]+ of [0-9]+",
    r"Use offset=[0-9]+ to continue",
    "Output " + "truncated",
    r"\[" + "truncated",
    "lines " + "truncated",
)


def check(revision=None):
    matches = subprocess.run(
        ["git", "grep", "-a", "-n", "-o", "-E", "|".join(PATTERNS),
         *([revision] if revision else []), "--"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if matches.returncode not in (0, 1):
        sys.stderr.write(matches.stderr.decode("utf-8", errors="replace"))
        return 1
    failed = matches.returncode == 0
    if failed:
        print("repository hygiene: captured tool-output notice(s); restore the complete file:",
              file=sys.stderr)
        sys.stderr.write(matches.stdout.decode("utf-8", errors="replace"))

    # The same offline parser serves CI and hooks; never fall back to site packages.
    archive = Path(__file__).resolve().parent / "vendor/pyyaml-6.0.3.zip"
    try:
        parser_bytes = archive.read_bytes()
    except OSError as error:
        print(f"repository hygiene: missing vendored YAML parser: {error}", file=sys.stderr)
        return 1
    if hashlib.sha256(parser_bytes).hexdigest() != "94ac21b4343e15647b3b5b5bd594aab5b7d3cd8d2ce17db15acb7260d489b6fc":
        print("repository hygiene: vendored YAML parser checksum mismatch", file=sys.stderr)
        return 1
    sys.path.insert(0, str(archive))
    try:
        import yaml
    except ImportError as error:
        print(f"repository hygiene: cannot load vendored YAML parser: {error}", file=sys.stderr)
        return 1
    command = (["git", "ls-tree", "-r", "--name-only", "-z", revision, "--", ".github/workflows/"]
               if revision else ["git", "ls-files", "-z", "--", ".github/workflows/"])
    paths = subprocess.run(command, stdout=subprocess.PIPE, check=True).stdout.split(b"\0")
    for raw in paths:
        if not raw:
            continue
        path = Path(raw.decode("utf-8"))
        if path.parent != Path(".github/workflows") or path.suffix not in {".yml", ".yaml"}:
            continue
        try:
            if revision:
                content = subprocess.run(["git", "show", f"{revision}:{path.as_posix()}"],
                                         stdout=subprocess.PIPE, check=True).stdout.decode("utf-8")
                source = io.StringIO(content)
            else:
                source = path.open(encoding="utf-8")
            with source:
                document = yaml.safe_load(source)
        except (yaml.YAMLError, UnicodeError, OSError) as error:
            mark = getattr(error, "problem_mark", None)
            line = mark.line + 1 if mark is not None else 1
            print(f"repository hygiene: {path}:{line}: invalid workflow YAML: {error}",
                  file=sys.stderr)
            failed = True
            continue
        if not isinstance(document, dict):
            print(f"repository hygiene: {path}:1: workflow must be a YAML mapping", file=sys.stderr)
            failed = True
    return int(failed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pre-push", action="store_true", help="validate every outgoing Git tip, not the working tree")
    args = parser.parse_args()
    # A local replacement ref must not hide the committed bytes being pushed.
    os.environ["GIT_NO_REPLACE_OBJECTS"] = "1"
    if not args.pre_push:
        return check()
    revisions = set()
    for line in sys.stdin:
        fields = line.split()
        if len(fields) != 4 or not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", fields[1]):
            print("repository hygiene: invalid pre-push ref input", file=sys.stderr)
            return 1
        if fields[1].strip("0"):
            revisions.add(fields[1])
    return int(any([check(revision) for revision in sorted(revisions)]))


if __name__ == "__main__":
    raise SystemExit(main())
