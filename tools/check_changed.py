#!/usr/bin/env python3
"""Check only what this branch changed: rustfmt, clippy and tests of the changed crates and their dependents.

Compares the working tree (committed, uncommitted and untracked files) with the merge-base against the base branch and
uses CI's crate selection (tools/ci/test_scope.py). A change outside every crate, such as Cargo.lock, .cargo/ or
.github/, selects the whole workspace. The launcher is its own workspace: tools/pr_ready.py checks it.
"""

import argparse
import shlex
import subprocess
import sys
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[1]))
from tools.ci import rust_changes, test_scope  # noqa: E402


def git(*arguments):
    return subprocess.check_output(["git", *arguments]).decode("utf-8")


def local_changes(base):
    """The merge-base with `base`, and every path changed since: committed, staged, unstaged and untracked."""
    merge_base = git("merge-base", base, "HEAD").strip()
    tracked = git("diff", "--name-only", "--no-renames", "-z", merge_base, "--").split("\0")
    untracked = git("ls-files", "--others", "--exclude-standard", "-z").split("\0")
    return merge_base, sorted({path for path in tracked + untracked if path})


def plan(workspace, paths):
    """([commands], [explanation lines]) for the changed paths."""
    chosen = workspace.scope(paths)
    if chosen is None:
        why = ["the whole workspace: a changed file belongs to no crate (Cargo.lock, .cargo/, .github/, tools/ ...)"]
        return [
            ["cargo", "fmt", "--all", "--", "--check"],
            ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
            ["cargo", "test", "--locked", "--workspace", "--lib", "--bins", "--tests"],
        ], why
    changed = set()
    for path in paths:
        changed |= workspace.owners(path) | workspace.readers(path)
    why = [f"changed: {', '.join(sorted(changed & chosen))}"]
    if chosen - changed:
        why.append(f"depend on them: {', '.join(sorted(chosen - changed))}")
    packages = [part for name in sorted(chosen) for part in ("-p", name)]
    fmt = [part for name in sorted(changed & chosen) for part in ("-p", name)]
    return [
        ["cargo", "fmt", *fmt, "--", "--check"],
        ["cargo", "clippy", *packages, "--all-targets", "--locked", "--", "-D", "warnings"],
        ["cargo", "test", "--locked", *packages, "--lib", "--bins", "--tests"],
    ], why


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/main", help="branch to compare with (default origin/main)")
    parser.add_argument("--dry-run", action="store_true", help="print the commands without running them")
    args = parser.parse_args()
    merge_base, paths = local_changes(args.base)
    if not rust_changes.needs_rust(paths):
        print("No Rust change: nothing to check.")
        return 0
    commands, why = plan(test_scope.Workspace(test_scope.cargo_metadata()), paths)
    print(f"{len(paths)} files changed since {merge_base[:9]}")
    print("\n".join(why))
    for command in commands:
        print("$ " + shlex.join(command))
        if not args.dry_run and subprocess.run(command).returncode:
            print("FAILED: " + shlex.join(command))
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
