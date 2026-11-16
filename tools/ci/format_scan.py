#!/usr/bin/env python3
"""The repository's formatting check, the one CI's `format` and `lint` jobs and `tools/pr_ready.py` run.

    python3 tools/ci/format_scan.py

`cargo fmt --all` reaches only the modules a `mod` line names, so it misses the gameplay modules the build derives
(`.game-area`), the API and vocabulary files a build script includes, and the generator's and wire loopback's
fixtures. Each of those is checked by path as well. Every check runs, so one run names every unformatted file; the
exit status is 1 if any of them failed. The launcher is a separate workspace with its own check.
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Every `.rs` file under these folders, checked one file at a time (no child modules followed).
TREES = ("crates/horizon-mod-api/src", "crates/horizon-mod-vocab/src", "crates/horizon-client/src/game")
# Files no `mod` line reaches.
DISCOVERED = ("crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/*.rs",
              "crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/*/*.rs",
              "crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback/*.rs")
RUSTFMT = ["rustfmt", "--edition", "2024", "--check", "--config", "skip_children=true"]
# Windows limits a command line to 32,767 characters; a chunk of paths stays well under it.
CHUNK = 200


def commands(root: Path) -> list[list[str]]:
    files = sorted(str(path.relative_to(root)) for tree in TREES for path in (root / tree).rglob("*.rs"))
    files += sorted(str(path.relative_to(root)) for pattern in DISCOVERED for path in root.glob(pattern))
    return [["cargo", "fmt", "--all", "--", "--check"]] + [
        RUSTFMT + files[start:start + CHUNK] for start in range(0, len(files), CHUNK)]


def main() -> int:
    failed = [command for command in commands(ROOT) if subprocess.run(command, cwd=ROOT).returncode != 0]
    if failed:
        print(f"format_scan: {len(failed)} formatting check(s) failed; the differences are above.", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
