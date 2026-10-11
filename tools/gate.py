#!/usr/bin/env python3
"""The check a branch must pass before it merges into the integration branch.

    python3 tools/gate.py               # everything
    python3 tools/gate.py --no-parity   # skip the vanilla harnesses
    python3 tools/gate.py --quick       # formatting, lints and Python tests only
    python3 tools/gate.py --known-failures mob_ai,light   # harnesses already failing on the base

Runs, in order, stopping at the first failure: the public-tree check,
rustfmt, clippy with warnings denied, the Rust tests, the Python tests, and
then every vanilla harness whose inputs changed (tools/parity.py). Harnesses
listed with --known-failures may still fail (they fail on the integration
branch too); any other failing harness fails the gate. CI runs the
same Rust and Python checks; the harnesses are the part CI cannot run.
The result is written to .local/gate.json.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
RESULT = ROOT / ".local" / "gate.json"


def steps(args):
    yield "public tree", [sys.executable, "tools/check_public_tree.py"]
    yield "rustfmt", ["cargo", "+stable", "fmt", "--all", "--", "--check"]
    yield "clippy", ["cargo", "+stable", "clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"]
    if args.quick:
        yield "python tests", [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-q"]
        return
    # Clippy has already compiled the examples; building them again for the
    # test run is most of the time and disk `cargo test --workspace` takes.
    yield "rust tests", ["cargo", "+stable", "test", "--locked", "--workspace", "--lib", "--bins", "--tests"]
    yield "doc tests", ["cargo", "+stable", "test", "--locked", "--workspace", "--doc"]
    yield "python tests", [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-q"]
    if not args.no_parity:
        yield "vanilla harnesses", [sys.executable, "tools/parity.py", "-j", str(args.jobs)]


def head():
    return subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--no-parity", action="store_true", help="skip the vanilla harnesses")
    parser.add_argument("--quick", action="store_true", help="only formatting, lints and Python tests")
    parser.add_argument("--known-failures", default="", help="comma-separated harnesses allowed to keep failing")
    parser.add_argument("-j", "--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 3), help="parallel harnesses")
    args = parser.parse_args()
    logs = ROOT / ".local" / "gate"
    logs.mkdir(parents=True, exist_ok=True)
    commit, dirty = head(), bool(subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, capture_output=True, text=True).stdout.strip())
    results = []
    passed = True
    for name, command in steps(args):
        started = time.monotonic()
        log = logs / (name.replace(" ", "-") + ".log")
        print(f"{name}...", end=" ", flush=True)
        if name == "vanilla harnesses":
            # Stream the runner's own progress; it keeps per-harness logs.
            status = subprocess.run(command, cwd=ROOT).returncode
            if status:
                known = set(filter(None, args.known_failures.split(",")))
                state = json.loads((ROOT / ".local/parity-runner/state.json").read_text())["harnesses"]
                failing = {harness for harness, record in state.items() if record["status"] != "pass"}
                new = sorted(failing - known)
                print(f"Failing harnesses: {', '.join(sorted(failing))}; new: {', '.join(new) or 'none'}")
                status = 1 if new else 0
        else:
            with log.open("w") as stream:
                status = subprocess.run(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT).returncode
        seconds = round(time.monotonic() - started)
        results.append({"step": name, "passed": status == 0, "seconds": seconds})
        if status:
            passed = False
            print(f"FAILED ({seconds}s)")
            if name != "vanilla harnesses":
                print("".join(log.read_text(errors="replace").splitlines(keepends=True)[-40:]))
                print(f"Full log: {log}")
            break
        print(f"ok ({seconds}s)")
    RESULT.write_text(json.dumps({"commit": commit, "dirty": dirty, "passed": passed, "quick": args.quick,
                                  "parity": not (args.quick or args.no_parity), "steps": results,
                                  "finished": time.strftime("%Y-%m-%dT%H:%M:%S")}, indent=1))
    print("Gate passed." if passed else "Gate failed.")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
