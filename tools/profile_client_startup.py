#!/usr/bin/env python3
"""Measure the resource-dependent Behaviours::load portion of client startup.

Build the behaviour_load_profile example in the checkout to be measured first:
    cargo build --release -p horizon-world --example behaviour_load_profile

Run the profiler from the repository root with the matching example binary.
This measures behaviour loading, not time to first rendered frame.
"""
import argparse
import json
import re
import statistics
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/examples/behaviour_load_profile"))
    parser.add_argument("--resources", type=Path, default=Path(".local/horizon-client/26.3"))
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be at least one")
    samples = []
    for _ in range(args.repeats):
        run = subprocess.run([str(args.binary.resolve()), str(args.resources.resolve())],
                             text=True, capture_output=True, check=True)
        values = {key: float(value) for key, value in
                  re.findall(r"^(registry_ms|behaviours_[0-2]_ms)=([\d.]+)$", run.stderr, re.M)}
        if len(values) != 4:
            raise RuntimeError(f"incomplete profile output: {run.stderr}")
        samples.append(values)
    summary = {key: {"median_ms": round(statistics.median(row[key] for row in samples), 3),
                     "samples_ms": [row[key] for row in samples]}
               for key in samples[0]}
    result = {"scope": "behaviour loader only; not full client startup",
              "binary": str(args.binary), "resources": str(args.resources), "repeats": args.repeats,
              "metrics": summary}
    print(json.dumps(result, indent=2))
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
