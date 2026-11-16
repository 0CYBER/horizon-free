#!/usr/bin/env python3
"""Move tests out of `verify` into the daily run: `#[ignore = "daily"]` on the test and its policy row.

    move_to_daily.py select MEASUREMENT.tsv > LIST.tsv   # which tests move, by the rule below
    move_to_daily.py apply LIST.tsv                      # mark them and write their rows
    move_to_daily.py restore LIST.tsv                    # undo apply: unmark them and drop their rows

LIST.tsv has one test per line, tab-separated: `PACKAGE TARGET TEST SOURCE REASON`. TARGET is the cargo target the
test is in (`lib`, `bin:NAME` or `test:NAME`, as the daily run selects it), TEST its libtest path, SOURCE the
`file:line` of its `fn`, REASON why it is a daily test (`slow`, `game-boot`, ...). `#` starts a comment line.

`select` reads a measurement of verify's tests (tab-separated with a header: package, target_kind, target, test,
source, max_seconds, ...; each test timed in-process by libtest's --report-time) and prints the tests that move:
- `slow`: its slower run took more than 2 seconds (the unit tier's limit; CONTRIBUTING.md "CI tiers");
- `game-boot`: it reaches a game-boot fixture (tools/ci/check_game_boot_tests.py, the same check verify-static
  runs, so after the move that check passes).
A test that is both is `slow`. Rows that are not Rust tests (documentation examples, Python) are reported, not
listed: an attribute cannot move them.

`apply` finds each test's `fn` at SOURCE (or, if the line moved, the only `fn` of that name in the file), checks
that its name is TEST's last segment, inserts `#[ignore = "daily"]` on its own line right after the test attribute
(`#[test]`, `#[tokio::test]`, ...) with that attribute's indentation, and writes the row
`PACKAGE TEST TARGET daily REASON` into tools/ci/ignored-tests/daily-<PACKAGE>.txt, keeping the rows sorted (one
file per package, sorted, so two pull requests that each move a test rarely touch the same lines). It never guesses:
a test it cannot place (a `fn` that is not there, a name that is a macro's, no test attribute on a line of its own)
is reported and skipped for a hand fix, and so is a test that is already `#[ignore]`d for another reason or listed
in another policy file. Running it again changes nothing: a test already marked `daily` only gets its missing row.
"""

import csv
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.ci import check_game_boot_tests, rust_tests  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("ignored-tests")
SLOW_SECONDS = 2.0
MARK = '#[ignore = "daily"]'
TEST_ATTRIBUTE = re.compile(r"^(\s*)#\[\s*(?:\w+\s*::\s*)*test\s*(?:\(.*\))?\s*\]\s*$")
FILE_HEADER = """\
# The daily tests of {package}: each is #[ignore = "daily"] in its source and runs once a day in the daily run,
# where it must pass (README.md). Rows: PACKAGE TEST TARGET daily REASON, sorted; tools/ci/move_to_daily.py writes them.
"""


def target_of(kind, name):
    """The daily run's target selector for a measured target kind and name, or None for a non-Rust test."""
    if kind in ("lib", "rlib", "cdylib", "proc-macro", "cdylib,rlib", "rlib,cdylib"):
        return "lib"
    if kind in ("bin", "test"):
        return f"{kind}:{name}"
    return None


def select(measurement, root=ROOT):
    """(list rows, skipped notes) from a measurement TSV, by the rule in this file's docstring."""
    names = check_game_boot_tests.fixtures()
    booting = {}  # file -> {fn line: chain}
    rows, skipped = [], []
    with open(measurement, newline="") as handle:
        for entry in csv.DictReader(handle, delimiter="\t"):
            slow = float(entry["max_seconds"]) > SLOW_SECONDS
            target = target_of(entry["target_kind"], entry["target"])
            path, _, line = entry["source"].rpartition(":")
            if target is None:
                if slow:
                    skipped.append(f"{entry['package']} {entry['test']}: a {entry['target_kind']} test over "
                                   f"{SLOW_SECONDS:g} s, not a Rust #[test]: not moved by an attribute")
                continue
            if not slow:
                if path not in booting:
                    booting[path] = {function.line: chain for function, chain in check_game_boot_tests.offenders(
                        (root / path).read_text(errors="replace"), names)} if (root / path).is_file() else {}
                if int(line) not in booting[path]:
                    continue
            rows.append((entry["package"], target, entry["test"], entry["source"], "slow" if slow else "game-boot"))
    return sorted(rows), skipped


def read_list(path):
    rows = []
    for number, line in enumerate(Path(path).read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 5 or not re.fullmatch(r"lib|(?:bin|test):[\w-]+", fields[1]) or ":" not in fields[3]:
            sys.exit(f"{path}:{number}: expected PACKAGE<TAB>TARGET<TAB>TEST<TAB>file:line<TAB>REASON")
        rows.append(tuple(fields))
    return rows


def policy_rows(folder):
    """(package, test) -> (file name, row text) of every row in the policy folder."""
    listed = {}
    for path in sorted(folder.glob("*.txt")):
        for line in path.read_text().splitlines():
            fields = line.split(" # ", 1)[0].split()
            if fields and not fields[0].startswith("#") and len(fields) >= 2:
                listed[fields[0], fields[1]] = (path.name, " ".join(fields))
    return listed


def place(lines, path, line, name):
    """The index of the line holding `fn name` (at `line`, else the file's only one), or a reason it can't."""
    declares = re.compile(r"\bfn\s+(?:r#)?" + re.escape(name) + r"\b")
    if 1 <= line <= len(lines) and declares.search(lines[line - 1]):
        return line - 1, None
    found = [index for index, text in enumerate(lines) if declares.search(text)]
    if len(found) == 1:
        return found[0], None
    at = lines[line - 1].strip() if 1 <= line <= len(lines) else "past the end of the file"
    return None, (f"{path}:{line} is not `fn {name}` ({at[:80]!r}); {len(found)} `fn {name}` in the file: a "
                  "macro-generated test or a moved one, fix by hand")


def mark(lines, index):
    """Insert MARK after the test attribute above lines[index]; returns 'marked', 'already', or a reason it can't."""
    attribute = None
    k = index - 1
    while k >= 0:
        text = lines[k].strip()
        if not (text.startswith("#[") or text.startswith("//")):
            break
        if TEST_ATTRIBUTE.match(lines[k]) and attribute is None:
            attribute = k
        k -= 1
    above = [lines[i].strip() for i in range(k + 1, index)]
    if MARK in above:
        return "already"
    if any(re.search(r"\bignore\b", text) for text in above if text.startswith("#[")):
        return "already #[ignore]d for another reason: refused"
    if attribute is None:
        return ("no test attribute on a line of its own above the fn (written on the fn's line, inside a "
                "multi-line attribute, or by a macro): fix by hand")
    indent = TEST_ATTRIBUTE.match(lines[attribute])[1]
    lines.insert(attribute + 1, indent + MARK)
    return "marked"


def write_row(folder, package, row):
    """Add `row` to daily-<package>.txt, keeping its rows sorted; False if it is already there."""
    path = folder / f"daily-{package}.txt"
    lines = path.read_text().splitlines() if path.exists() else FILE_HEADER.format(package=package).splitlines()
    if row in lines:
        return False
    header = [line for line in lines if line.startswith("#") or not line.strip()]
    rows = sorted([line for line in lines if line.strip() and not line.startswith("#")] + [row])
    path.write_text("\n".join(header + rows) + "\n")
    return True


def apply(rows, root=ROOT, folder=POLICY):
    """Mark and list every row; returns (moved, already, skipped) where skipped holds (row, reason)."""
    listed = policy_rows(folder)
    moved, already, skipped = [], [], []
    by_file = {}
    for row in rows:
        by_file.setdefault(row[3].rpartition(":")[0], []).append(row)
    for path, file_rows in sorted(by_file.items()):
        source = root / path
        if not source.is_file():
            skipped.extend((row, f"{path} does not exist") for row in file_rows)
            continue
        text = source.read_text()
        lines = text.splitlines()
        # Bottom first, so an insertion never shifts a line still to place.
        placed = []
        for row in file_rows:
            package, target, test, location, reason = row
            expected = f"{package} {test} {target} daily {reason}"
            if (package, test) in listed and listed[package, test][1] != expected:
                skipped.append((row, f"already listed in {listed[package, test][0]}: refused"))
                continue
            index, why = place(lines, path, int(location.rpartition(":")[2]), test.rsplit("::", 1)[-1])
            if why:
                skipped.append((row, why))
                continue
            placed.append((index, row, expected))
        # One mark per function: a source file compiled into two targets has a row for each.
        outcomes = {}
        for index, row, expected in sorted(placed, reverse=True):
            if index not in outcomes:
                outcomes[index] = mark(lines, index)
            outcome = outcomes[index]
            if outcome not in ("marked", "already"):
                skipped.append((row, outcome))
                continue
            added = write_row(folder, row[0], expected)
            (moved if outcome == "marked" or added else already).append(row)
        new = "\n".join(lines) + ("\n" if text.endswith("\n") else "")
        if new != text:
            source.write_text(new)
    return moved, already, skipped


def restore(rows, root=ROOT, folder=POLICY):
    """Undo `apply` for every row: drop the mark above its `fn` and its row; returns (restored, skipped)."""
    restored, skipped = [], []
    by_file = {}
    for row in rows:
        by_file.setdefault(row[3].rpartition(":")[0], []).append(row)
    for path, file_rows in sorted(by_file.items()):
        source = root / path
        text = source.read_text()
        lines = text.splitlines()
        for row in file_rows:
            package, target, test, location, reason = row
            index, why = place(lines, path, int(location.rpartition(":")[2]), test.rsplit("::", 1)[-1])
            k = index - 1 if index is not None else -1
            while k >= 0 and lines[k].strip() != MARK and lines[k].strip().startswith(("#[", "//")):
                k -= 1
            if why or k < 0 or lines[k].strip() != MARK:
                skipped.append((row, why or "no daily mark above the fn"))
                continue
            del lines[k]
            policy = folder / f"daily-{package}.txt"
            kept = [line for line in policy.read_text().splitlines()
                    if line != f"{package} {test} {target} daily {reason}"]
            policy.write_text("\n".join(kept) + "\n")
            restored.append(row)
        new = "\n".join(lines) + ("\n" if text.endswith("\n") else "")
        if new != text:
            source.write_text(new)
    return restored, skipped


def main(argv):
    if len(argv) == 2 and argv[0] == "select":
        rows, skipped = select(argv[1])
        print("# PACKAGE\tTARGET\tTEST\tSOURCE\tREASON  (tools/ci/move_to_daily.py select; the rule is in its docstring)")
        for row in rows:
            print("\t".join(row))
        for note in skipped:
            print(f"skipped: {note}", file=sys.stderr)
        print(f"{len(rows)} tests selected, {len(skipped)} skipped", file=sys.stderr)
        return 0
    if len(argv) == 2 and argv[0] == "apply":
        moved, already, skipped = apply(read_list(argv[1]))
        for row, why in skipped:
            print(f"skipped {row[0]} {row[2]}: {why}")
        print(f"{len(moved)} tests moved to the daily run, {len(already)} already there, {len(skipped)} skipped")
        return 0
    if len(argv) == 2 and argv[0] == "restore":
        restored, skipped = restore(read_list(argv[1]))
        for row, why in skipped:
            print(f"skipped {row[0]} {row[2]}: {why}")
        print(f"{len(restored)} tests back in verify, {len(skipped)} skipped")
        return 0
    sys.exit(__doc__)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
