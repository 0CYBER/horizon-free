#!/usr/bin/env python3
"""Reject local game artifacts even when somebody uses git add -f."""

import argparse
from pathlib import PurePosixPath
import subprocess
import sys

PRIVATE_ROOTS = {".local", "decompiled", "minecraft", "assets", "run", "logs"}
PRIVATE_EXTENSIONS = {".jar", ".class", ".jfr", ".hprof", ".java"}
ORIGINAL_JDK_TOOLS = {"tools/JfrSummary.java", "tests/java/JfrFixture.java"}


def prohibited(path):
    parsed = PurePosixPath(path)
    # Original JDK-only tools are an exact allowlist; recovered game Java stays private.
    if path in ORIGINAL_JDK_TOOLS:
        return False
    # Original Horizon mod sources may also be tracked.
    if parsed.suffix == ".java" and path.startswith(("mods/horizon/src/", "mods/horizon-lab/src/")):
        return not ("/dev/horizonlab/" in path)
    return bool(parsed.parts and parsed.parts[0] in PRIVATE_ROOTS) or parsed.suffix.lower() in PRIVATE_EXTENSIONS


def paths_in_repository(history=False):
    if not history:
        result = subprocess.check_output(["git", "ls-files", "-z"])
        return result.decode("utf-8").split("\0")
    result = subprocess.check_output(["git", "log", "--all", "--format=", "--name-only", "-z"])
    return [path.lstrip("\n") for path in result.decode("utf-8").split("\0")]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--history", action="store_true", help="Also inspect paths in all local Git history")
    args = parser.parse_args()
    rejected = sorted({path for path in paths_in_repository(args.history) if path and prohibited(path)})
    if rejected:
        print("Private game/profiling artifacts must stay outside Git:", file=sys.stderr)
        for path in rejected:
            print(f"  {path}", file=sys.stderr)
        return 1
    print("Public tree check passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
