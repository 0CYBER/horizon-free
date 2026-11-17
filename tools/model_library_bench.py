#!/usr/bin/env python3
"""Compare saved model_library_bench baseline/candidate binaries on prepared resources.

Build both with the same profile, then run under the shared heavy-work lock:
  python3 tools/model_library_bench.py BASELINE CANDIDATE RESOURCE_ROOT
Fresh processes include all asset/item setup. Two warmups per binary precede
10 alternating samples. Page caches are warm; this does not simulate cold disk.
"""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import tempfile


def parse_row(text):
    """Every `key=value` numeric token a benchmark printed, with a named error for a malformed one.

    The old `[0-9.]+` class dropped negative values (e.g. `delta_ms=-3.0`) without a word and matched
    `v=1.2.3`, leaving a bare `float()` to raise after all samples had been collected.
    """
    row = {}
    for key, value in re.findall(r"(\w+)=(\S+)", text):
        try:
            row[key] = float(value)
        except ValueError as error:
            raise ValueError(f"cannot parse {key}={value!r} in benchmark output: {text!r}") from error
    return row


def summarize(rows):
    """Per-key figures over every sample that reported that key, not just the ones in the first sample."""
    keys = sorted({key for row in rows for key in row})
    return {key: {
        "median": statistics.median(row[key] for row in rows if key in row),
        "samples": [row[key] for row in rows if key in row],
    } for key in keys}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("resources", type=Path)
    args = parser.parse_args()
    binaries = {name: str(getattr(args, name).resolve()) for name in ("baseline", "candidate")}
    samples = {name: [] for name in binaries}
    for iteration in range(12):
        order = list(binaries) if iteration % 2 == 0 else list(reversed(binaries))
        for label in order:
            with tempfile.TemporaryFile(mode="w+t") as output:
                command = [binaries[label], str(args.resources)]
                with subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT) as process:
                    _, status, usage = os.wait4(process.pid, 0)
                    process.returncode = os.waitstatus_to_exitcode(status)
                    output.seek(0)
                    text = output.read()
                    if process.returncode:
                        raise subprocess.CalledProcessError(process.returncode, command, text)
            row = parse_row(text)
            row["peak_rss_kib"] = usage.ru_maxrss
            print(json.dumps({"iteration": iteration, "label": label, **row}), flush=True)
            if iteration >= 2:
                samples[label].append(row)
    for label, rows in samples.items():
        print(label, json.dumps(summarize(rows)), flush=True)


if __name__ == "__main__":
    main()
