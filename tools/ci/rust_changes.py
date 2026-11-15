#!/usr/bin/env python3
"""Select CI checks from one fail-closed Git diff.

The default answers Rust selection; --launcher uses the launcher's dependency paths.
--all emits the shared Rust, launcher and tooling decisions in one answer.
server_build: whether verify's clippy of the server build has anything new to check (needs_server_build).
"""

import argparse
import subprocess
import sys
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools import release_train  # noqa: E402

# The root workspace's manifest and toolchain files: the launcher's path crates inherit from them.
ROOT_RUST = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}
# The check itself: a change to it runs it.
LAUNCHER_CHECK = {".github/workflows/ci.yml", "tools/ci/rust_changes.py", "tools/release_train.py"}


# The Python under tools/ and tests/: verify's Python steps and the parity jobs run it, and no Rust build or test reads
# it (Rust does read tools/ci/*.sh, so those still count). This script and the
# module it imports decide whether Rust runs at all, so a change to them runs it.
RUST_DECIDES = {"tools/ci/rust_changes.py", "tools/release_train.py"}


def needs_rust(paths):
    return any(not (
        path.startswith("docs/") or path.endswith(".md")
        or path.startswith(("tools/", "tests/")) and path.endswith(".py") and path not in RUST_DECIDES
    ) for path in paths)


# verify's second clippy, the server build (horizon-client without default features): what it checks differently from
# the first is the feature-gated code of the client crates and the features the manifests choose. Pull requests that
# change neither skip it; main's pushes and the merge queue always run it.
SERVER_BUILD_CHECK = {".github/workflows/ci.yml", "tools/ci/rust_changes.py"}


def needs_server_build(paths):
    return any(path.startswith("crates/horizon-client") or path.endswith("Cargo.toml") or path in ROOT_RUST
               or path.startswith(".cargo/") or path in SERVER_BUILD_CHECK for path in paths)


def needs_launcher(paths):
    return any(path in ROOT_RUST or path in LAUNCHER_CHECK or (
        path.startswith(tuple(release_train.LAUNCHER_PATHS))
        and not path.startswith(tuple(release_train.LAUNCHER_IGNORED))
        and not path.endswith(".md")
    ) for path in paths)


# Stands for "everything changed" when no diff can be computed: a root manifest every check and shard selector
# treats as touching all of them.
UNKNOWN_CHANGE = "Cargo.lock"


def needs_tooling(changes):
    paths = [path for _, path in changes]
    return needs_rust(paths) or any(
        path.startswith(("tools/", "tests/", ".github/", "docs/modding/", "roadmap/"))
        or path in {"README.md", "ROADMAP.md", "docs/README.md"}
        # Tests check linked target existence, even when they do not read its contents.
        or status != "M"
        for status, path in changes
    )


def selections(changes):
    paths = [path for _, path in changes]
    return {"rust_required": needs_rust(paths),
            "launcher_required": needs_launcher(paths),
            "tooling_required": needs_tooling(changes)}


def changed_entries(base, head):
    # Disable rename folding: retain both deleted and added paths, including source moved into docs.
    command = ["git", "diff", "--name-status", "--no-renames", "-z", f"{base}...{head}", "--"]
    for fetch_base in (False, True):
        # The event's base can be newer than the checked-out history (main moved on): fetch it once.
        if fetch_base:
            subprocess.run(["git", "fetch", "-q", "--no-tags", "origin", base], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        diff = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
        if diff.returncode == 0:
            fields = diff.stdout.decode("utf-8").split("\0")[:-1]
            return list(zip(fields[::2], fields[1::2]))
    # Never an error and never "nothing changed": assume everything did.
    print(f"rust_changes: no diff for {base}...{head}; treating everything as changed", file=sys.stderr)
    return [("?", UNKNOWN_CHANGE)]


def changed_paths(base, head):
    return [path for _, path in changed_entries(base, head)]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--launcher", action="store_true", help="answer for the launcher instead")
    mode.add_argument("--all", action="store_true", help="emit the Rust, launcher and tooling selections at once")
    args = parser.parse_args()
    changes = changed_entries(args.base, args.head)
    paths = [path for _, path in changes]
    if args.all:
        for name, required in selections(changes).items():
            print(f"{name}={str(required).lower()}")
    else:
        needs = needs_launcher if args.launcher else needs_rust
        print(f"required={str(needs(paths)).lower()}")
        print(f"server_build={str(needs_server_build(paths)).lower()}")
