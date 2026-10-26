#!/usr/bin/env python3
"""Keep verify's Rust steps for any change beyond documentation.

--launcher: whether the change is built into the launcher, its own workspace (release_train.LAUNCHER_PATHS).
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


def needs_rust(paths):
    return any(not (path.startswith("docs/") or path.endswith(".md")) for path in paths)


def needs_launcher(paths):
    return any(path in ROOT_RUST or path in LAUNCHER_CHECK or (
        path.startswith(tuple(release_train.LAUNCHER_PATHS))
        and not path.startswith(tuple(release_train.LAUNCHER_IGNORED))
        and not path.endswith(".md")
    ) for path in paths)


def changed_paths(base, head):
    # Renames must include deletions too: moving source into docs still affects Rust.
    output = subprocess.check_output([
        "git", "diff", "--name-only", "--no-renames", "-z", f"{base}...{head}", "--",
    ])
    return [path for path in output.decode("utf-8").split("\0") if path]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    parser.add_argument("--launcher", action="store_true", help="answer for the launcher instead")
    args = parser.parse_args()
    needs = needs_launcher if args.launcher else needs_rust
    print(f"required={str(needs(changed_paths(args.base, args.head))).lower()}")
