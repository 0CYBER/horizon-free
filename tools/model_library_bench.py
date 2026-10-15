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
            row = {key: float(value) for key, value in re.findall(r"(\w+)=([0-9.]+)", text)}
            row["peak_rss_kib"] = usage.ru_maxrss
            print(json.dumps({"iteration": iteration, "label": label, **row}), flush=True)
            if iteration >= 2:
                samples[label].append(row)
    for label, rows in samples.items():
        print(label, json.dumps({key: {
            "median": statistics.median(row[key] for row in rows),
            "samples": [row[key] for row in rows],
        } for key in rows[0]}), flush=True)


if __name__ == "__main__":
    main()
