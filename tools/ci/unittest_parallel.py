#!/usr/bin/env python3
"""Run tests/ as `python -m unittest discover -s tests` does, one module per process, several at a time.

verify's Python step: the suite took about 2 minutes in one process while the runner's other cores idled. Each module
runs alone in its own interpreter (`discover -s tests -p <file>`, so imports resolve exactly as in one discovery).
Modules in importable subpackages are found as discover finds them and run under their dotted names, each once.
A failing module's whole output is printed, and the exit status is 1 when any module failed.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
RAN = re.compile(r"^Ran (\d+) tests? in ", re.M)
# unittest's status when a module has no test to run (Python 3.12+): not a failure.
NO_TESTS = 5


# discover's import setup: tests/ first on sys.path, so modules import by their dotted names under it.
LOAD = "import sys, unittest; sys.path.insert(0, 'tests'); unittest.main(module=None)"


def modules(folder, package=""):
    """The modules `discover -s tests` loads: test*.py files, descending into folders that are packages."""
    for path in sorted(folder.iterdir()):
        if path.is_file() and path.name.startswith("test") and path.suffix == ".py":
            yield package + path.stem, path
        elif path.is_dir() and (path / "__init__.py").is_file():
            yield from modules(path, f"{package}{path.name}.")


def run(module):
    name, _path = module
    start = time.monotonic()
    result = subprocess.run(
        [sys.executable, "-c", LOAD, name],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace",
    )
    ran = RAN.search(result.stdout)
    return name, result.returncode, result.stdout, int(ran.group(1)) if ran else 0, time.monotonic() - start


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1, help="modules at a time (default: every core)")
    args = parser.parse_args()
    # The biggest files first, so a long module doesn't start last and finish alone.
    found = sorted(modules(ROOT / "tests"), key=lambda module: (-module[1].stat().st_size, module[0]))
    start = time.monotonic()
    failed, tests, times = [], 0, []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for future in as_completed([pool.submit(run, module) for module in found]):
            name, status, output, ran, seconds = future.result()
            tests += ran
            times.append((seconds, name))
            ok = status in (0, NO_TESTS)
            print(f"{'ok' if ok else 'FAILED':6} {name} ({ran} tests, {seconds:.1f} s)", flush=True)
            if not ok:
                failed.append(name)
                print(output, flush=True)
    print(f"\nRan {tests} tests in {len(found)} modules, {args.jobs} at a time, in {time.monotonic() - start:.1f} s")
    print("Slowest: " + ", ".join(f"{name} {seconds:.1f} s" for seconds, name in sorted(times, reverse=True)[:5]))
    if failed:
        print(f"FAILED: {', '.join(sorted(failed))}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
