"""Run a preparation boundary privately; publish only fixed reason codes, never log text."""

import argparse
import errno
import json
import os
import re
from pathlib import Path
import subprocess

STEPS = ("preparer-build", "resource-export", "seed-extras")
# These are diagnostic hints, not a claim about the root cause. Never interpolate matches.
REASONS = (
    ("no space left on device", "disk-full"),
    ("read-only file system", "read-only-filesystem"),
    ("permission denied", "permission-denied"),
    ("outofmemoryerror", "java-out-of-memory"),
    ("could not resolve host", "network-dns"),
    ("connection timed out", "network-timeout"),
    ("no such file or directory", "missing-file"),
)


def run(step: str, log: Path, command: list[str]) -> int:
    status = 1
    reason = "unclassified"
    last_error = "redacted-unclassified"
    safe_line = "unclassified: see private log (not retained on per-job runners)"
    # The cache has no content-key marker: .entries lists names, not a digest.
    # Validate environment strings once before they can enter public diagnostics.
    expected = os.environ.get("HORIZON_DIAGNOSTIC_EXPECTED_KEY", "")
    expected = expected if re.fullmatch(r"[0-9a-f]{16}", expected) else "unavailable-or-invalid"
    state = os.environ.get("HORIZON_DIAGNOSTIC_CACHE_STATE", "")
    state = state if state in ("missing-entry-list", "missing-horizon-client") else "unavailable"
    try:
        with log.open("wb") as output:
            result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, check=False)
        status = result.returncode
        if status == 0:
            return 0
        # Bounded private read. Unknown or late failures stay explicitly unclassified.
        with log.open("rb") as source:
            sample = source.read(1024 * 1024).lower()
        reason = next((code for marker, code in REASONS if marker.encode() in sample), reason)
        # Inspect only a bounded suffix, dropping its possibly partial first line.
        # Report a fixed classification of the last nonempty line, never its payload.
        with log.open("rb") as source:
            size = source.seek(0, 2)
            source.seek(max(0, size - 4096))
            tail = source.read(4096)
        if size > 4096:
            tail = tail.partition(b"\n")[2]
        lines = [line for line in tail.splitlines() if line.strip()]
        if lines:
            last_error = next((code for marker, code in REASONS if marker.encode() in lines[-1].lower()), last_error)
            # Only a WHOLE line equal to one fixed marker may be published.
            # Prefixes, paths, tokens, suffixes and file excerpts always get redacted.
            if lines[-1].lower() in (marker.encode() for marker, _ in REASONS):
                safe_line = lines[-1].decode("ascii")
    except OSError as error:
        reason = {errno.ENOSPC: "disk-full", errno.EROFS: "read-only-filesystem",
                  errno.EACCES: "permission-denied", errno.ENOENT: "missing-file"}.get(error.errno, "os-error")
    # step is validated once by argparse; status is an integer from subprocess, not log data.
    print(f"::error::preparation step={step} exit={status} reason_hint={reason} "
          f"expected_key={expected} found_key=not-recorded cache_state={state} "
          f"last_line_classification={last_error}", flush=True)
    print(safe_line, flush=True)
    diagnostic = dict(schema_version=1, step=step, exit_status=status, reason_hint=reason,
                      expected_key=expected, found_key="not-recorded", cache_state=state,
                      last_line_classification=last_error)
    try:
        log.with_suffix(".diagnostic.json").write_text(json.dumps(diagnostic) + "\n")
    except OSError:
        print("preparation diagnostic sidecar unavailable", flush=True)
    return status if status > 0 else 128 - status


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("step", choices=STEPS)
    parser.add_argument("log", type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not args.command:
        parser.error("a command is required")
    return run(args.step, args.log, args.command)


if __name__ == "__main__":
    raise SystemExit(main())
