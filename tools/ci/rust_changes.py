#!/usr/bin/env python3
"""Keep verify's Rust steps for any change beyond website content or documentation."""

import argparse
import subprocess


def needs_rust(paths):
    # horizon-theme's Rust tests check that the generated website tokens match.
    return any(path == "website/src/tokens.css" or not (
        path.startswith(("website/", "docs/")) or path.endswith(".md")
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
    args = parser.parse_args()
    print(f"required={str(needs_rust(changed_paths(args.base, args.head))).lower()}")
