#!/usr/bin/env python3
"""Compile the machine sample at PIN against this checkout's API (the daily leg machine-sample.yml).

A pull request that changes the API the sample uses updates PIN to a sample commit that builds against it.
"""
import os
from pathlib import Path
import subprocess
import sys
import time
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PIN = "ba91a01eddfdfae53a93cc93965f7767ca7f65c2"


def main():
    revision = PIN
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="machine-sample-", dir=ROOT.parent) as directory:
        check(Path(directory), revision)
    elapsed = time.monotonic() - started
    summary = f"Machine sample `{revision}`: {elapsed:.1f} seconds ({elapsed / 60:.2f} runner minutes).\n"
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as output:
            output.write(summary)


def check(directory, revision):
    sample = directory / "sample"
    subprocess.run(["git", "clone", "--no-checkout", "https://github.com/arismoko/horizon-machine-sample", str(sample)], check=True)
    subprocess.run(["git", "fetch", "origin", revision], cwd=sample, check=True)
    subprocess.run(["git", "checkout", "--detach", revision], cwd=sample, check=True)
    # The sample names the API by the path `cargo horizon-mod` fetches the signed SDK to. This checkout's own API
    # source goes there instead, so the check builds the sample against this PR's API with no SDK release.
    subprocess.run([sys.executable, str(ROOT / "tools/sdk_api_source.py"), "build", str(sample / ".horizon/sdk/api")],
                   cwd=ROOT, check=True)
    subprocess.run(["cargo", "check", "--all-targets"], cwd=sample, check=True)


if __name__ == "__main__":
    main()
