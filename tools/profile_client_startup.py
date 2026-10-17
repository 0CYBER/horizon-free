#!/usr/bin/env python3
"""Compare four JSON loading sequences in the client startup table phase.

Build first with `cargo build --release -p horizon-client --example shared_json_startup_profile`.
Modes: baseline uses independent public loaders, pr607 reproduces a01d47f2,
shared uses the paired wrapper, and deferred retains the projection until the
normal behaviour stage. All modes use one optimized executable. This covers
Assets::load plus BlockFacts and Behaviours loading; it excludes full startup,
rendering, resource-pack resolution and first frame. Peak RSS includes Assets.
"""
import argparse
import json
import os
import statistics
import subprocess
import tempfile
from pathlib import Path


def run(binary, resources, mode):
    with tempfile.TemporaryFile(mode="w+t") as output:
        proc = subprocess.Popen([str(binary), str(resources), mode], stdout=output, stderr=subprocess.PIPE)
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        _, stderr = proc.communicate()
        if os.waitstatus_to_exitcode(status):
            raise RuntimeError(f"{mode}: {stderr.decode(errors='replace')}")
        output.seek(0)
        fields = dict(line.rstrip().split("=", 1) for line in output if "=" in line)
    return {
        "phase_ms": float(fields["phase_ms"]),
        "total_ms": float(fields["total_ms"]),
        "cpu_ms": (usage.ru_utime + usage.ru_stime) * 1000.0,
        "peak_rss_kib": usage.ru_maxrss,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/examples/shared_json_startup_profile"))
    parser.add_argument("--resources", type=Path, default=Path(".local/horizon-client/26.3"))
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    binary, resources = args.binary.resolve(), args.resources.resolve()
    if not binary.is_file() or not resources.is_dir():
        parser.error("binary or resources missing; build the example and prepare resources first")
    samples = {"baseline": [], "pr607": [], "shared": [], "deferred": []}
    for i in range(args.repeats):
        for mode in (("baseline", "pr607", "shared", "deferred") if i % 2 == 0 else ("deferred", "shared", "pr607", "baseline")):
            samples[mode].append(run(binary, resources, mode))
    result = {
        "scope": "Assets + block facts + behaviours loader only; excludes full startup",
        "resources": str(resources),
        "repeats": args.repeats,
        "metrics": {mode: {key: {"median": round(statistics.median(row[key] for row in rows), 3),
                                  "samples": [round(row[key], 3) for row in rows]}
                           for key in rows[0]} for mode, rows in samples.items()},
    }
    data = json.dumps(result, indent=2)
    print(data)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(data + "\n")


if __name__ == "__main__":
    main()
