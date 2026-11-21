#!/usr/bin/env python3
"""Make every test that reruns its own test executable select the test whatever its ignore state (arismoko/horizon#4022).

A test that reruns a test executable as a child (any `Command::new(...)` passing libtest's `--exact NAME`: its own,
from `current_exe()`, or a copy of it)
and passes neither `--ignored` nor `--include-ignored` runs nothing in the child once NAME is `#[ignore]`d: libtest
reports the test ignored, exits 0, and the parent passes having checked nothing. That is every such test the daily
tier moved. This script inserts `"--include-ignored"` before the first `"--exact"` of each such rerun (in an
`.args([...])` list, or `.arg("--include-ignored")` before `.arg("--exact")`), so the child runs the named test as
before when it is not ignored and runs it too when it is. A rerun is read from its `Command::new(`
to the `.status()`, `.output()` or `.spawn()` that runs it, or to the end of the helper that returns it built (at
most 40 lines). Running it again changes nothing.

Usage: reexec_include_ignored.py [--check]   (--check: list the reruns that lack it, change nothing; exit 1 if any)
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
START = re.compile(r"Command::new\(")
END = re.compile(r"\.\s*(?:status|output|spawn)\s*\(\s*\)")
SELECTS = re.compile(r'"--(?:include-)?ignored"')
EXACT = '"--exact"'


def reruns(lines):
    """(first line, last line) index pairs of every rerun that names `--exact`."""
    found = []
    for index, line in enumerate(lines):
        if START.search(line):
            indent = len(line) - len(line.lstrip())
            for end in range(index, min(index + 40, len(lines))):
                # Run where it is built, or returned built (a helper ending in `}` at or above its indentation).
                closes = end > index and lines[end].strip() == "}" and len(lines[end]) - len(lines[end].lstrip()) < indent
                if END.search(lines[end]) or closes:
                    if any(EXACT in text for text in lines[index:end + 1]):
                        found.append((index, end))
                    break
    return found


def missing(lines):
    """The reruns among `lines` that select by `--exact` without `--ignored` or `--include-ignored`."""
    return [(start, end) for start, end in reruns(lines)
            if not any(SELECTS.search(text) for text in lines[start:end + 1])]


def fix(lines):
    for start, end in missing(lines):
        for index in range(start, end + 1):
            if EXACT in lines[index]:
                if re.search(r'\.arg\(\s*"--exact"\s*\)', lines[index]):
                    lines[index] = lines[index].replace('.arg("--exact")', '.arg("--include-ignored").arg("--exact")', 1)
                elif lines[index].strip() == f"{EXACT},":
                    indent = lines[index][:len(lines[index]) - len(lines[index].lstrip())]
                    lines[index] = f'{indent}"--include-ignored",\n{lines[index]}'
                else:
                    lines[index] = lines[index].replace(EXACT, f'"--include-ignored", {EXACT}', 1)
                break
    return lines


def sources():
    listing = subprocess.run(["git", "ls-files", "-z", "--", "*.rs", ":!launcher"], cwd=ROOT, capture_output=True,
                             text=True, check=True)
    return [ROOT / path for path in filter(None, listing.stdout.split("\0"))]


def main(check):
    problems = 0
    for path in sources():
        text = path.read_text(encoding="utf-8")
        if "current_exe" not in text:
            continue
        lines = text.split("\n")
        found = missing(lines)
        problems += len(found)
        for start, _ in found:
            print(f"{path.relative_to(ROOT)}:{start + 1}: reruns its test executable by --exact without "
                  "--include-ignored (or --ignored), so the child runs nothing once the test is ignored")
        if found and not check:
            path.write_text("\n".join(fix(lines)), encoding="utf-8")
    print(f"{problems} reruns {'lack' if check else 'gained'} --include-ignored")
    return 1 if check and problems else 0


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["--check"]):
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1:] == ["--check"]))
