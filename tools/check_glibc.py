#!/usr/bin/env python3
"""Keep Linux release executables runnable on Ubuntu 22.04 (docs/ci-runners.md)."""
import argparse
import os
import re
import shutil
import subprocess
import sys

# Ubuntu 22.04's libc. Newer builders must never silently raise this floor.
FLOOR = (2, 35)


def incompatible_versions(report):
    """Version requirements, not the builder's installed libc, determine compatibility."""
    incompatible = set()
    needs = False
    for line in report.splitlines():
        if line.startswith("Version "):
            needs = line.startswith("Version needs section")
        if not needs:
            continue
        for name in re.findall(r"\bName:\s+(GLIBC_[A-Za-z0-9_.]+)", line):
            version = re.fullmatch(r"GLIBC_(\d+(?:\.\d+)+)", name)
            # ABI tags (e.g. GLIBC_ABI_DT_RELR, added in 2.36) and private versions
            # are not an Ubuntu 22.04 public ABI. Unknown tags must not pass silently.
            if version is None or tuple(map(int, version[1].split("."))) > FLOOR:
                incompatible.add(name)
    return sorted(incompatible)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", nargs="+")
    args = parser.parse_args()
    readelf = shutil.which("readelf")
    if readelf is None:
        parser.error("readelf is required (install binutils)")
    failed = False
    for binary in args.binaries:
        result = subprocess.run([readelf, "--wide", "--version-info", "--", binary], text=True, capture_output=True,
                                env={**os.environ, "LC_ALL": "C"})
        if result.returncode:
            print(result.stderr, file=sys.stderr, end="")
            failed = True
            continue
        incompatible = incompatible_versions(result.stdout)
        if incompatible:
            print(f"{binary}: requires {', '.join(incompatible)}; Linux releases must support glibc 2.35 (Ubuntu 22.04)", file=sys.stderr)
            failed = True
        else:
            print(f"{binary}: glibc requirements fit Ubuntu 22.04")
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
