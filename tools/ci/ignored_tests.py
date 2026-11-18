#!/usr/bin/env python3
"""Run every library test, ignored or not, with the prepared vanilla data, against a list of known failures.

Usage: ignored_tests.py [--without-data] SHARD...  (HORIZON_TEST_RESOURCES points at the prepared 26.3 data)
       ignored_tests.py --needs VARIABLE (--one-process-each | --list) [--jobs N]

A SHARD is a package, or one leg of it written `PACKAGE#NofM` (tools/ci/ignored_shards.py splits the biggest
package so one process does not hold every test's memory). A leg runs the tests whose name hashes to its
number, so every test runs in exactly one leg, and only judges the list's entries for those same tests.
An unqualified horizon-client-lib runs two such selections consecutively on the same runner, releasing
retained fixture memory between processes without adding a matrix job.

Each package's library tests run with `--include-ignored`: the #[ignore]d ones that need the data, and the
ones that return early without it, which `verify` (no data) passes without checking anything. The packages
in INTEGRATION_TESTS also run their `tests/` integration tests. The job fails when a test fails that
no file in tools/ci/ignored-tests/ lists, when a listed test passes (take it off its file: the lists only
shrink), and when a listed test no longer exists. A test marked `flaky` there
may pass or fail; one marked `harness` is a parity harness's step, run by its harness with the
environment it sets, and is skipped here, as is one marked `manual` (run by hand, by its parent test, or by the
workflow its row names); one marked `needs` reads an input only its parity harness provides
(REFERENCE names the environment variable): without that variable it is refused by name at the start of the
shard, and with it, it runs and must pass; one marked `gpu` needs a graphics adapter and runs only where
the runner has a Vulkan driver (an ICD file), and must pass there.

`needs-plain` is like `needs`, but selects a plain test without `--ignored`.

`--without-data` is for a machine with no prepared vanilla data (GitHub's runners): it leaves out the daily rows whose
reason is `resources` or `game-boot` and every `harness`, `needs`, `needs-plain` and `gpu` row, and prints how many
tests it left out. The tests left out run nowhere until a machine with the data runs the shards without the flag.

A row marked `daily` (`PACKAGE TEST TARGET daily REASON`, in `daily-<package>.txt`) is a test moved out of
`verify` into the daily run with `#[ignore = "daily"]` (tools/ci/move_to_daily.py): not a known failure, it must
pass, and its row must name a test that still runs. A package's shard runs the daily rows of the targets it
already runs (its library; every target for the packages in INTEGRATION_TESTS); the shard `daily-rows`
(tools/ci/ignored_shards.py; `daily-rows-launcher` for the launcher's) runs every other daily row, one `cargo test -p PACKAGE TARGET -- --ignored --exact
TEST...` per package and target, from the package's own workspace: a launcher row runs from launcher/ against its
own lockfile (`--locked`).

`--needs VARIABLE --one-process-each` runs the tests marked `needs VARIABLE`, each alone in its own process
(`--list` prints those commands instead), and fails unless every run reports exactly one test passed: a
renamed test would otherwise match nothing and pass. Every test runs, then the step fails if any did.
`--daily-tests PACKAGE TARGET` prints the tests one target's daily rows list, one per line (the Windows daily job
runs horizon-mod-package's with `-- --ignored --exact`).
`--report FILE` also writes the run's verdict as JSON for the daily run's summary (tools/ci/daily_summary.py): each
problem's package, test identity, kind, panic location and policy file, the tests that passed, and the runs that
did not complete with why (stopped()). It is written before each run too, naming that run as in progress, so a job
stopped at its time limit still leaves one. It holds only names and numbers this script prints anyway, never test
output.

Every reader refuses a malformed row, a test listed twice, a folder with no rows or with a file other than `*.txt`
and README.md, and a file at the retired single-list path (tools/ci/check_ignored_test_rows.py checks this alone).

Runtime diagnostics expose identities and allowlisted panic categories, never arbitrary assertion
payloads: the prepared data is Mojang's. Compile/list failures expose bounded compiler headings.
"""

import argparse
import json
import os
import errno
import io
from functools import lru_cache
import re
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path
from typing import NamedTuple

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import lab
from tools.ci import ignored_shards

ROOT = Path(__file__).resolve().parents[2]

# One file per topic, read in file-name order (README.md there gives the row format).
POLICY = Path(__file__).with_name("ignored-tests")
POLICY_PATH = "tools/ci/ignored-tests"  # as messages name it
# The single list these files replaced: a file there again fails every reader, naming the folder.
RETIRED = Path(__file__).with_name("ignored-tests-known-failures.txt")
RESULT = re.compile(r"^test (\S+)(?: - should panic)? \.\.\. (ok|FAILED)$")
SUMMARY = re.compile(r"^test result: \S+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured;")
# Cargo's line as a test executable starts: `Running unittests src/lib.rs (…)`, `Running tests/x.rs (…)`, or a
# target whose source lives in another package's folder (`Running ../horizon-client/tests/lan_bot.rs (…)`).
TARGET = re.compile(r"^\s+Running (?:unittests )?\S+ \(")
COUNT = re.compile(r"^running (\d+) tests?$")
PANIC = re.compile(r"^thread '([^']+)'.* panicked at (.+):$")
# The Rust runtime's own words when it aborts a process: fixed text, no payload, safe to show.
ABORTS = (
    (re.compile(r"^thread '[^']*'.* has overflowed its stack$"), "stack overflow"),
    (re.compile(r"^thread '[^']*'.* panicked while processing panic"), "a panic while panicking"),
    (re.compile(r"^panicked while panicking"), "a panic while panicking"),
    (re.compile(r"^thread caused non-unwinding panic\. aborting\.$"), "a panic that cannot unwind"),
    (re.compile(r"^panic in a (?:function|destructor) (?:that cannot unwind|during cleanup)"), "a panic that cannot unwind"),
    (re.compile(r"^memory allocation of \d+ bytes failed$"), "a failed memory allocation"),
)
# Packages whose integration tests (`tests/*.rs`) run too, beside their library tests.
INTEGRATION_TESTS = {"horizon-client"}
LEG = re.compile(r"^(?P<package>[^#]+)#(?P<number>\d+)of(?P<count>\d+)$")
LISTED = re.compile(r"^(\S+): test$")
# A daily row's TARGET: the library, or one binary or integration test (cargo's --bin NAME, --test NAME).
DAILY_TARGET = re.compile(r"^(?:lib|(?:bin|test):[\w-]+)$")
# Why a test is a daily test (CONTRIBUTING.md "CI tiers"): slower than 2 s, or what it needs.
DAILY_REASONS = ("slow", "game-boot", "resources", "child-process", "network", "wasm-fixture")
# Who runs a `manual` row's test, never the daily shards: a person, its parent test (a child process it starts), or
# a workflow of .github/workflows/ by file name.
MANUAL_RUNNERS = re.compile(r"^(?:by-hand|subprocess|[\w.-]+\.ya?ml)$")
WORKFLOWS = Path(__file__).resolve().parents[2] / ".github" / "workflows"
# The daily reasons that need the prepared vanilla data, and the other marks that do (`--without-data` leaves them out).
DATA_REASONS = ("resources", "game-boot")
DATA_MARKS = ("harness", "gpu")
ICD_DIRS = ("/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d")
# The largest tracked Rust file read to check a panic location's line: above every source in the tree (the largest
# are near 0.7 MB), so a location in any tracked source of any package is reported, while a blob stays bounded.
SOURCE_BYTES = 2_000_000


def compile_diagnostics(output):
    """List mode never executes tests. Show headings, not source excerpts or arbitrary Cargo output."""
    headings = []
    for line in output.splitlines():
        if re.match(r"^error(?:\[E\d+\])?:", line):
            # Compiler/proc-macro diagnostics can contain resource-derived text even
            # without quotes. Only fixed reason forms cross this boundary.
            code = re.match(r"^error(\[E\d+\])?:", line)[1] or ""
            reason = line.split(":", 1)[1].strip()
            safe = reason if reason in {
                "mismatched types", "type annotations needed", "unexpected closing delimiter",
                "unclosed delimiter", "expected item, found keyword", "aborting due to previous error",
            } else "custom diagnostic payload withheld"
            if reason.startswith("linking with "):
                safe = "linker failure; command/output withheld"
                if status := re.search(r"(?:^|: )exit status: (\d{1,5})(?:\D|$)", reason):
                    safe += f"; exit status={status[1]}"
            elif reason.startswith("failed to run custom build command for "):
                safe = "custom build command failure; command/output withheld"
            headings.append(f"error{code}: {safe}")
        elif re.match(r"^\s*--> [\w./-]+:\d+:\d+$", line):
            headings.append(line)
    for line in headings[:20]:
        print("  compiler: " + line[:300] + (" [shortened]" if len(line) > 300 else ""))
        if line.strip().startswith("--> "):
            panic_source(line.strip()[4:], kind="compiler")
    if len(headings) > 20:
        print(f"  compiler diagnostics truncated: {len(headings) - 20} more headings")
    if not headings:
        print("  no recognized compiler diagnostic; non-diagnostic output withheld")


@lru_cache(maxsize=32)
def source_lines(path):
    """Only immutable tracked Rust blobs, never a working-tree or resource file."""
    if not re.fullmatch(r"(?:crates|launcher)/[A-Za-z0-9_./-]+\.rs", path):
        return None
    if any(part in ("", ".", "..", "target", ".local") for part in path.split("/")):
        return None
    # ls-tree mode rejects symlinks and bounds blobs before git-show allocates output.
    entry = subprocess.run(["git", "ls-tree", "-l", "HEAD", "--", path],
                           capture_output=True, text=True, errors="replace")
    fields = entry.stdout.split()
    if entry.returncode or len(fields) != 5 or fields[0] not in ("100644", "100755") or fields[1] != "blob":
        return None
    if not fields[3].isdigit() or int(fields[3]) > SOURCE_BYTES:
        return None
    blob = subprocess.run(["git", "show", f"HEAD:{path}"], capture_output=True, text=True, errors="replace")
    return blob.stdout.splitlines() if blob.returncode == 0 else None


def panic_source(location, kind="panic"):
    found = re.fullmatch(r"(.+\.rs):(\d+):(\d+)", location)
    lines = source_lines(found[1]) if found else None
    if lines is None or not 1 <= int(found[2]) <= len(lines):
        print(f"  {kind} source unavailable (not a bounded tracked Rust location)")
        return
    line = lines[int(found[2]) - 1]
    # Prefix and escape controls so checked-in text cannot inject workflow commands.
    line = line.encode("unicode_escape").decode("ascii")
    print(f"  {kind} source: " + line[:300] + (" [shortened]" if len(line) > 300 else ""))


def has_vulkan():
    """Whether this machine has a Vulkan driver for wgpu to find."""
    return bool(os.environ.get("VK_ICD_FILENAMES")) or any(
        Path(d).is_dir() and any(Path(d).glob("*.json")) for d in ICD_DIRS
    )


def parse_shard(shard):
    """(package, leg number or None, leg count or None) of a shard argument."""
    found = LEG.match(shard)
    if not found:
        return shard, None, None
    number, count = int(found["number"]), int(found["count"])
    if not 1 <= number <= count:
        sys.exit(f"{shard}: leg {number} of {count} doesn't exist")
    return found["package"], number, count


def in_leg(test, number, count):
    """Whether `test` belongs to leg `number` of `count` (every test to exactly one leg; all, with no legs)."""
    return number is None or zlib.crc32(test.encode()) % count == number - 1


class Row(NamedTuple):
    package: str
    test: str
    reference: str
    mark: str | None
    file: str  # `ignored-tests/<topic>.txt`, for messages
    reason: str | None = None  # a `daily` row's reason; its REFERENCE is its target


def listed_entries():
    """Every row of the policy files, in file-name order; exits on a malformed row or a test listed twice."""
    if RETIRED.exists():
        sys.exit(f"tools/ci/{RETIRED.name} is retired: write its rows into {POLICY_PATH}/<topic>.txt "
                 f"(one file per policy or tracked failure; {POLICY_PATH}/README.md gives the format)")
    if not POLICY.is_dir():
        sys.exit(f"{POLICY_PATH}/ is missing: it holds the ignored-test policy rows")
    if stray := sorted(path.name for path in POLICY.iterdir() if path.suffix != ".txt" and path.name != "README.md"):
        sys.exit(f"{POLICY_PATH}/ holds only <topic>.txt files and README.md; not read: {', '.join(stray)}")
    seen = {}
    rows = []
    for path in sorted(POLICY.glob("*.txt")):
        file = f"{POLICY_PATH}/{path.name}"
        for number, text in enumerate(path.read_text().splitlines(), 1):
            line = text.split(" # ", 1)[0].split()
            if not line or line[0].startswith("#"):
                continue
            if line[3:4] == ["manual"]:
                if (len(line) != 4 or not MANUAL_RUNNERS.match(line[2]) or " # " not in text
                        or not text.split(" # ", 1)[1].strip()):
                    sys.exit(f"{file}:{number}: expected `PACKAGE TEST WHO manual # WHY`, WHO `by-hand`, `subprocess` "
                             "or a workflow file name, and WHY saying why it never runs in the daily shards")
                if MANUAL_RUNNERS.match(line[2]) and line[2].endswith((".yml", ".yaml")) and not (WORKFLOWS / line[2]).is_file():
                    sys.exit(f"{file}:{number}: no workflow .github/workflows/{line[2]}")
            elif line[3:4] == ["daily"]:
                if len(line) != 5 or not DAILY_TARGET.match(line[2]) or line[4] not in DAILY_REASONS:
                    sys.exit(f"{file}:{number}: expected `PACKAGE TEST TARGET daily REASON`, TARGET `lib`, "
                             f"`bin:NAME` or `test:NAME`, REASON one of {', '.join(DAILY_REASONS)}")
            elif len(line) < 3 or line[3:] not in ([], ["flaky"], ["harness"], ["gpu"], ["needs"], ["needs-plain"]):
                sys.exit(f"{file}:{number}: expected `PACKAGE TEST REFERENCE [flaky|harness|gpu|needs|needs-plain|daily|manual]`")
            if (line[0], line[1]) in seen:
                sys.exit(f"{file}:{number}: {line[0]} {line[1]} is already listed in {seen[line[0], line[1]]}; "
                         "a test belongs to one file")
            seen[line[0], line[1]] = f"{file}:{number}"
            rows.append(Row(line[0], line[1], line[2], line[3] if len(line) > 3 else None, file,
                            line[4] if len(line) > 4 else None))
    if not rows:
        sys.exit(f"{POLICY_PATH}/ has no rows")
    return rows


def known_failures():
    """(package, test) -> its mark (None, "flaky", "harness", "gpu" or "manual"), from the policy files: the tests a
    shard judges by their row, as a known failure (None, flaky, gpu) or by skipping them (harness, manual)."""
    return {(row.package, row.test): row.mark for row in listed_entries()
            if row.mark not in {"needs", "needs-plain", "daily"}}


def daily_rows():
    """The rows marked `daily`: tests moved out of verify that the daily run must run and pass."""
    return [row for row in listed_entries() if row.mark == "daily"]


def daily_tests(package, target):
    """The tests of `package`'s `target` (`lib`, `bin:NAME`, `test:NAME`) that its daily rows list, sorted: what a
    job outside the shards runs with `-- --ignored --exact`, so it runs no other ignored test (a `manual` one)."""
    return sorted(row.test for row in daily_rows() if row.package == package and row.reference == target)


def shard_runs(package, target):
    """Whether `package`'s own shard runs the tests of `target` (`lib`, `bin:NAME`, `test:NAME`)."""
    return package in ignored_shards.SHARDS and (
        target == "lib"
        or package in INTEGRATION_TESTS and target.startswith(("bin:", "test:"))
    )


@lru_cache(maxsize=None)
def workspace_features(workspace="."):
    """{package: its features as the workspace at `workspace` resolves them}: what its `--workspace` build compiles.

    `cargo test -p PACKAGE` alone resolves only that package's own needs, which leaves out a test behind a feature
    another member turns on (or a target's required-features), so a daily-rows run passes these explicitly."""
    metadata = subprocess.run(["cargo", "metadata", "--format-version", "1"], capture_output=True, text=True,
                              check=True, cwd=ROOT / workspace)
    data = json.loads(metadata.stdout)
    names = {package["id"]: package["name"] for package in data["packages"]}
    members = set(data["workspace_members"])
    return {names[node["id"]]: sorted(node["features"]) for node in data["resolve"]["nodes"] if node["id"] in members}


def daily_rows_groups(rows):
    """The daily rows no package shard runs, as {(package, target): [test, ...]}: the `daily-rows` shard's runs."""
    groups = {}
    for row in rows:
        if not shard_runs(row.package, row.reference):
            groups.setdefault((row.package, row.reference), []).append(row.test)
    return {key: sorted(tests) for key, tests in sorted(groups.items())}


def harness_inputs():
    """(package, test) -> the environment variable it reads that only its parity harness provides."""
    return {(row.package, row.test): row.reference for row in listed_entries() if row.mark in {"needs", "needs-plain"}}


def needs_commands(variable, jobs=None):
    """One `cargo test` command per test marked `needs variable`, each selecting exactly that test."""
    commands = [
        ["cargo", "test", *([f"-j{jobs}"] if jobs else []), "-p", row.package, "--lib", "--", *(["--ignored"] if row.mark == "needs" else []), "--exact", row.test]
        for row in listed_entries() if row.mark in {"needs", "needs-plain"} and row.reference == variable
    ]
    if not commands:
        sys.exit(f"no `needs` row in {POLICY_PATH}/ names {variable}")
    return commands


def write_report(path, report):
    """The daily summary's hand-off (--report): identities, locations and policy only."""
    if path:
        Path(path).write_text(json.dumps(report, indent=1, sort_keys=True) + "\n")


def run_one_process_each(variable, jobs=None, report=None):
    """Run each test that needs `variable` in its own process; 1 unless every run reports exactly one passed test."""
    if not os.environ.get(variable):
        sys.exit(f"{variable} is unset: these tests need it")
    failed = []
    verdict = {"needs": variable, "problems": [], "passed": [], "incomplete": []}
    for command in needs_commands(variable, jobs):
        # needs_commands ends each command with the test's exact name, after `-p PACKAGE`.
        entry = {"package": command[command.index("-p") + 1], "test": safe_identity(command[-1])}
        print(f"$ {shlex.join(command)}", flush=True)
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        passed = False
        for line in process.stdout:
            print(line, end="", flush=True)
            passed |= line.startswith("test result: ok. 1 passed;")
        if process.wait() or not passed:
            print(f"::error::expected 1 passing test from: {shlex.join(command)}", flush=True)
            failed.append(command)
            verdict["problems"].append({**entry, "kind": "failed", "panic": None, "policy": None})
        else:
            verdict["passed"].append(entry)
    write_report(report, verdict)
    print(f"{variable}: {len(failed)} of the runs failed" if failed else f"{variable}: every run passed its one test")
    return 1 if failed else 0


# Budgets include UTF-8 newlines; summary metadata is outside the payload budget.
FAILURE_BYTES = 500
FAILURE_TOTAL_BYTES = 128_000
DIAGNOSTIC_LINES = 100_000
DIAGNOSTIC_LINE_CHARS = 4096
DIAGNOSTIC_CONTEXTS = 4096
_active_budget = None
# Each failed test's tracked panic location (`path:line:col`) from the last failure_diagnostics, for --report.
_panic_locations = {}


def thread_identity(name):
    """A thread's name as a test identity, a short plain name (`Server thread`), or withheld."""
    if "::" in name:
        return f"test {safe_identity(name)}"
    if re.fullmatch(r"[A-Za-z0-9 <>_.-]{1,40}", name):
        return f"thread '{name}'"
    return "a thread (name withheld)"


# The longest test identity shown: above every test path in the tree (the longest are near 160 characters), so a
# real name is never cut, while a line that only looks like one cannot carry an unbounded payload.
IDENTITY_CHARS = 300


def safe_identity(name):
    # Rust path segments only, not arbitrary libtest payloads or GitHub tokens, and bounded in length.
    if len(name) > IDENTITY_CHARS:
        return f"identity withheld (longer than {IDENTITY_CHARS} characters)"
    if re.fullmatch(r"(?:r#)?[^\W\d]\w*(?:::(?:r#)?[^\W\d]\w*)*", name) and not re.search(
            r"(?:gh[pousr]_|github_pat_|token|secret)", name, re.IGNORECASE):
        return name
    return "identity withheld (unclassified test name)"


@lru_cache(maxsize=1)
def tracked_rust_paths():
    listing = subprocess.run(["git", "ls-tree", "-rl", "HEAD", "--", "crates", "launcher"],
                             capture_output=True, text=True, errors="replace")
    paths = set()
    if listing.returncode == 0:
        for line in listing.stdout.splitlines():
            fields = line.split()
            if (len(fields) == 5 and fields[0] in ("100644", "100755") and
                    fields[1] == "blob" and fields[3].isdigit() and int(fields[3]) <= SOURCE_BYTES and
                    re.fullmatch(r"(?:crates|launcher)/[A-Za-z0-9_./-]+\.rs", fields[4]) and
                    not any(part in ("", ".", "..", "target", ".local") for part in fields[4].split("/"))):
                paths.add(fields[4])
    return paths


# What an `Err` names, never what it says: only a std::io::Error Debug value (`Os { code, kind, .. }`,
# `Kind(..)`, `Custom { kind, .. }`) names anything here: its shape, an io::ErrorKind from std's list and an OS
# error code. Quoted strings inside it are removed before any field is read; any other payload (a string, an
# unquoted message, another error type) names nothing. Messages, values and paths stay withheld.
IO_ERROR_SHAPE = re.compile(r"^(Os|Simple|SimpleMessage|Custom|Kind)\s*[({]")
IO_ERROR_KINDS = frozenset("""
    NotFound PermissionDenied ConnectionRefused ConnectionReset HostUnreachable NetworkUnreachable
    ConnectionAborted NotConnected AddrInUse AddrNotAvailable NetworkDown BrokenPipe AlreadyExists WouldBlock
    NotADirectory IsADirectory DirectoryNotEmpty ReadOnlyFilesystem StaleNetworkFileHandle InvalidInput
    InvalidData TimedOut WriteZero StorageFull NotSeekable QuotaExceeded FileTooLarge ResourceBusy
    ExecutableFileBusy Deadlock CrossesDevices TooManyLinks InvalidFilename ArgumentListTooLong Interrupted
    Unsupported UnexpectedEof OutOfMemory InProgress Other Uncategorized
""".split())
QUOTED = re.compile(r'"(?:[^"\\]|\\.)*"')
ERROR_KIND = re.compile(r"(?:\bkind: |^Kind\()([A-Z][A-Za-z]{0,63})\b")
OS_ERROR = re.compile(r"^Os \{ code: (\d{1,5})\b")


def unwrap_error_shape(value):
    """`kind ...` for a std::io::Error Debug value, or None when the payload is anything else."""
    if not (shape := IO_ERROR_SHAPE.match(value)):
        return None
    fields = QUOTED.sub('""', value)
    kinds = [shape[1]]
    if (kind := ERROR_KIND.search(fields)) and kind[1] in IO_ERROR_KINDS:
        kinds.append(kind[1])
    if code := OS_ERROR.match(fields):
        kinds.append(f"os error {int(code[1])} {errno.errorcode.get(int(code[1]), 'unknown')}")
    return "kind " + ", ".join(kinds)


def bounded_record(record, remaining):
    raw = record.encode("utf-8")
    shortened = len(raw) > FAILURE_BYTES
    if shortened:
        marker = b" [shortened: per-test byte budget]\n"
        raw = raw[:max(0, FAILURE_BYTES - len(marker))].decode("utf-8", errors="ignore").encode("utf-8") + marker
    if len(raw) > remaining[0]:
        return False, False
    print(raw.decode("utf-8"), end="")
    remaining[0] -= len(raw)
    return True, shortened


def failure_diagnostics(output):
    """One record per failed result occurrence, including repeated names across targets.

    Panic contexts use libtest's name model and the first panic for that name.
    They cannot disambiguate identical names in different targets.
    """
    failures = []
    contexts = {}
    context = []
    aborts = []
    panic_reason_pending = False
    skipped_lines = 0
    skipped_contexts = 0
    stream = io.StringIO(output)
    for index, line in enumerate(stream):
        if index >= DIAGNOSTIC_LINES:
            skipped_lines += 1 + sum(1 for _ in stream)
            break
        if len(line) > DIAGNOSTIC_LINE_CHARS:
            skipped_lines += 1
            panic_reason_pending = False
            continue
        line = line.rstrip("\r\n")
        abort = next((label for pattern, label in ABORTS if pattern.match(line.strip())), None)
        if abort:
            thread = re.match(r"thread '([^']*)'", line.strip())
            # The Rust runtime aborts (SIGABRT, signal 6) after each of these messages.
            abort = f"(signal 6: {abort})" + (f" on {thread_identity(thread[1])}" if thread else
                                              "; fatal thread unknown, see the last started tests below")
            if abort not in aborts:
                aborts.append(abort)
        if RESULT.match(line) or SUMMARY.match(line) or TARGET.match(line):
            panic_reason_pending = False
        if panic_reason_pending and line.strip():
            context[:] = [item for item in context if item != "  panic reason missing from captured output"]
            # No free-form panic text or assertion operands cross the resource boundary.
            reason = next((label for prefix, label in (
                ("assertion `left == right` failed", "assertion equality failed"),
                ("assertion `left != right` failed", "assertion inequality failed"),
                ("assertion failed:", "assertion condition failed"),
                ("called `Result::unwrap()`", "Result::unwrap on an error"),
                ("called `Option::unwrap()`", "Option::unwrap on None"),
                ("index out of bounds", "index out of bounds"),
                ("attempt to", "arithmetic failure"),
            ) if line.strip().startswith(prefix)), "custom panic")
            withheld_text = "payload withheld (resource privacy)"
            if reason == "Result::unwrap on an error":
                if shape := unwrap_error_shape(line.strip().split(" value: ", 1)[-1]):
                    reason += f" ({shape})"
                    withheld_text = "rest of payload withheld (resource privacy)"
            if arithmetic := re.fullmatch(r"attempt to ((?:add|subtract|multiply|divide|negate|shift left|shift right) with overflow|(?:divide|calculate the remainder) by zero)", line.strip()):
                reason = "attempt to " + arithmetic[1]
            context.append(f"  panic reason: {reason}; {withheld_text}")
            panic_reason_pending = False
            continue
        if found := RESULT.match(line):
            if found[2] == "FAILED":
                failures.append(found[1])
        if found := PANIC.match(line):
            context = []
            if found[1] not in contexts:
                if len(contexts) < DIAGNOSTIC_CONTEXTS:
                    contexts[found[1]] = context
                else:
                    skipped_contexts += 1
                    continue
            location = re.fullmatch(r"(.+\.rs):(\d+):(\d+)", found[2])
            path = location[1] if location else ""
            if any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in path):
                path = ""
            # game-discovery emits canonical absolute #[path] declarations. Only
            # this checkout's exact resolved prefix is authoritative; never guess
            # another checkout's prefix or search for a matching path suffix.
            prefix = str(Path(__file__).resolve().parents[2]) + os.sep
            if path.startswith(prefix):
                path = path[len(prefix):]
            # Rust cross-crate source paths can contain lexical ../ segments.
            # Never resolve an untrusted path against the filesystem, and never
            # allow a traversal to leave the root, even if it later returns.
            parts = []
            for part in path.split("/"):
                if part == "..":
                    if not parts:
                        parts = []
                        break
                    parts.pop()
                elif part not in ("", "."):
                    parts.append(part)
            path = "/".join(parts) if not path.startswith("/") else ""
            lines = source_lines(path) if path in tracked_rust_paths() else None
            if (location and lines is not None and
                    1 <= int(location[2]) <= len(lines) and int(location[3]) >= 1):
                context.append(f"  panicked at {path}:{location[2]}:{location[3]}")
            else:
                context.append("  panic location withheld (not a bounded tracked Rust location)")
            context.append("  panic reason missing from captured output")
            panic_reason_pending = True
    remaining = _active_budget if _active_budget is not None else [FAILURE_TOTAL_BYTES]
    # What killed the process comes first: a test that libtest reported FAILED ran to its end, so it is never
    # the one that took the process down.
    for abort in aborts:
        print(f"FATAL: process aborted {abort}")
    if aborts and failures:
        print("ordinary failures (the process kept running after them):")
    shown = shortened = missing = withheld = 0
    identities_withheld = locations_withheld = payloads_withheld = reasons_shown = 0
    for name in failures:
        identity = safe_identity(name)
        details = contexts.get(name)
        if details is None:
            # No panic of its own: what panicked on threads that are not tests (a server or worker thread).
            details = []
            for thread, other in contexts.items():
                if "::" not in thread and thread not in failures:
                    details += ["  on a non-test thread:"] + other
            details = details or ["  panic context missing from captured output"]
        missing += any("missing from captured output" in item for item in details)
        withheld += identity != name or any("withheld" in item for item in details)
        identities_withheld += identity != name
        locations_withheld += any("panic location withheld" in item for item in details)
        payloads_withheld += any("payload withheld" in item for item in details)
        reasons_shown += any(item.startswith("  panic reason:") for item in details)
        _panic_locations[name] = next((item.removeprefix("  panicked at ") for item in details
                                       if item.startswith("  panicked at ")), None)
        record = f"  failed test: {identity}\n" + "\n".join(details) + "\n"
        emitted, cut = bounded_record(record, remaining)
        shown += emitted
        shortened += cut
    if not failures and contexts:
        # No failed test named: every thread's panic, unattributed.
        details = []
        for thread, other in contexts.items():
            details += [f"  panic on {thread_identity(thread)}:"] + other
        emitted, cut = bounded_record("  no failed test named; panics seen:\n" + "\n".join(details) + "\n", remaining)
        shortened += cut
    print(f"failure diagnostics: {shown} shown; {len(failures) - shown} failing tests not shown "
          f"(total byte budget {FAILURE_TOTAL_BYTES}); {shortened} records shortened "
          f"(per-test byte budget {FAILURE_BYTES}); {missing} missing context; {withheld} withheld context/identity; "
          f"{reasons_shown} classified panic reasons; {identities_withheld} identities withheld; "
          f"{locations_withheld} locations withheld; {payloads_withheld} payloads withheld (expected privacy filtering); "
          f"{skipped_lines} lines skipped (line/count bounds; failure count unknown there); "
          f"{skipped_contexts} contexts skipped (retention bound)")


RUNNING_LONG = re.compile(r"^test (\S+) has been running for over 60 seconds$")
CRASH_NAMES = 20
# The job's cgroup (v2): the whole job's memory, compilation and file cache included, as the data-leg lines print it.
CGROUP = Path("/sys/fs/cgroup")
# Why the last run() returned None, for --report: names and numbers only (stopped()).
_stopped = {}


def cgroup_memory():
    """The job cgroup's memory peak and limit in bytes (None where unreadable or unlimited) and its out-of-memory
    kills so far (None where unreadable)."""
    def read(name):
        try:
            return (CGROUP / name).read_text()
        except OSError:
            return ""
    peak, limit = read("memory.peak").strip(), read("memory.max").strip()
    kills = re.search(r"^oom_kill (\d+)$", read("memory.events"), re.M)
    return {"memory_peak_bytes": int(peak) if peak.isdigit() else None,
            "memory_max_bytes": int(limit) if limit.isdigit() else None,
            "oom_kills": int(kills[1]) if kills else None}


def stopped(reason, started, kills_before, process=None, results=(), expected=(), ended_by=None):
    """Record why a run did not complete (`_stopped`), and return None: the run's verdict.

    The reason; what ended the test process (`signal 9 (SIGKILL)`, `exit status 3`); cargo's exit; the wall time
    since the run started; the job's memory peak and limit and the out-of-memory kills during the run; the tests
    still running past 60 s (libtest's own notice; a test that started less than 60 s before the end is not known)
    and how many listed tests never reported. Test names pass safe_identity."""
    memory = cgroup_memory()
    output = process.stdout if process is not None else ""
    long = dict.fromkeys(found[1] for line in output.splitlines() if (found := RUNNING_LONG.match(line.strip())))
    still = [name for name in long if name not in results]
    kills = memory.pop("oom_kills")
    _stopped.clear()
    _stopped.update({
        "reason": reason, "ended_by": ended_by, "cargo_exit": process.returncode if process is not None else None,
        "wall_seconds": round(time.monotonic() - started), **memory,
        "oom_kills": kills - kills_before if kills is not None and kills_before is not None else None,
        "still_running": [safe_identity(name) for name in still[:CRASH_NAMES]],
        "still_running_more": max(0, len(still) - CRASH_NAMES),
        "never_reported": len(set(expected) - set(results)) if expected else None,
    })


def crash_report(package, output, expected, results):
    """After a crash: the tests that ran long and the listed ones that never reported, by name only."""
    long = []
    for line in output.splitlines():
        if (found := RUNNING_LONG.match(line.strip())) and found[1] not in results and found[1] not in long:
            long.append(found[1])
    if long:
        print(f"{package}: last started tests, still running past 60 s when it crashed: "
              + ", ".join(safe_identity(name) for name in long[:CRASH_NAMES]))
    unreported = sorted(set(expected) - set(results))
    if unreported:
        shown = ", ".join(safe_identity(name) for name in unreported[:CRASH_NAMES])
        more = f" (and {len(unreported) - CRASH_NAMES} more)" if len(unreported) > CRASH_NAMES else ""
        print(f"{package}: {len(unreported)} listed tests never reported (started or not yet started): {shown}{more}")


def missing_result_framing(output, name):
    """Describe bounded framing hints, never line contents or recovered results.

    Text searches can also find prefix identities or incidental mentions; counts
    describe candidate framing only and cannot establish that a test ran.
    """
    counts = {"ansi-prefix": 0, "interleaved-record": 0, "split-status": 0}
    cursor = matches = 0
    seen = set()
    while matches < 8 and (position := output.find(name, cursor)) >= 0:
        matches += 1
        cursor = position + len(name)
        start = output.rfind("\n", 0, position) + 1
        if start in seen:
            continue
        seen.add(start)
        end = output.find("\n", position)
        if end < 0:
            end = len(output)
        prefix = output[max(start, position - 512):position]
        suffix = output[cursor:min(end, cursor + 512)]
        counts["ansi-prefix"] += bool(re.search(r"\x1b\[[0-?]*[ -/]*[@-~]", prefix))
        counts["interleaved-record"] += prefix.count("test ") > 1 or "test " in suffix
        following = output[end + 1:end + 513].split("\n", 1)[0].strip()
        counts["split-status"] += suffix.rstrip().endswith("...") and following in {"ok", "FAILED"}
    framing = ",".join(key for key, count in counts.items() if count)
    framing = framing or ("unclassified-candidate" if seen else "absent-intact-identity")
    metadata = "; ".join(f"{key}={count}" for key, count in counts.items())
    return f"framing={framing}; candidate_lines={len(seen)}; {metadata}; sampled_matches={matches}; sample_limit=8"


def listed_result_records(line, expected, max_name_length):
    """Literal, bounded parsing of listed identities in interleaved libtest output.

    Prefix output is ignored, but every matching listed result is counted:
    a second prefixed record must not escape duplicate detection. Ambiguous
    suffixes are reported to the caller, never guessed or printed. A slow-test
    notice or other mention without a result status is not a result record.
    """
    cursor = line.find("test ")
    while cursor >= 0:
        start = cursor + len("test ")
        end = line.find(" ", start, start + max_name_length + 1)
        if end >= 0:
            name = line[start:end]
            if name in expected:
                for panic in ("", " - should panic"):
                    for status in ("ok", "FAILED", "ignored"):
                        ending = f"{panic} ... {status}"
                        if line.startswith(ending, end):
                            after = end + len(ending)
                            boundary = (after == len(line) or line.startswith("test ", after)
                                        or line.startswith(" test ", after))
                            yield name, status if boundary else "ambiguous"
        cursor = line.find("test ", start)


def run(package, skip, number=None, count=None, only=None):
    """Each test's result in `package`'s ignored library tests but `skip` (and, with a leg, the other legs'), or None if they didn't build.

    `only=(target, tests)` runs just those ignored tests of one target (`lib`, `bin:NAME`, `test:NAME`) instead.
    When it returns None, `_stopped` says why (stopped())."""
    started = time.monotonic()
    kills_before = cgroup_memory()["oom_kills"]
    # Resource-backed fixtures need a fresh home, not the runner user's game files.
    with tempfile.TemporaryDirectory(prefix="horizon-ignored-") as scratch:
        homes = {}
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
            home = Path(scratch) / key.lower()
            home.mkdir()
            homes[key] = home
        for key in ("HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            homes[key] = Path(scratch) / key.lower()
        env = lab.game_env(**homes)
        if package == "horizon-client-lib":
            # Worldgen must meet its real-time deadlines without disabling test assertions.
            env["CARGO_PROFILE_TEST_OPT_LEVEL"] = "3"
        # --no-fail-fast: a target with accepted known failures must not stop the targets after it.
        # Owner-approved stopgap; arismoko/horizon#2944 removes this when the legs run under 15 minutes.
        client_half = package == "horizon-client" and count == 2 and number in (1, 2)
        command = ["cargo", "test", "-p", package, "--lib", *(["--bins", "--tests"] if package in INTEGRATION_TESTS else []), "--no-fail-fast"]
        workspace = "."
        if only:
            target = only[0]
            # A package of another workspace (the launcher's) runs from that workspace, against its own lockfile.
            workspace = ignored_shards.workspace_members()[package][1]
            features = workspace_features(workspace).get(package, [])
            command = ["cargo", "test", *(["--locked"] if workspace != "." else []), "-p", package,
                       *(["--features", ",".join(features)] if features else []),
                       *(["--lib"] if target == "lib" else [f"--{target.split(':')[0]}", target.split(":", 1)[1]])]
        if client_half:
            # The runner's own cap (tools/ci/cargo_jobs.py --cap) when set, else the old fixed one.
            command.append(f"--jobs={os.environ.get('CI_JOBS', '8')}")
        # What the package's tests need built first (ignored_shards.PREPARE: the command verify runs too).
        if prepare := ignored_shards.PREPARE.get(package):
            built = subprocess.run(prepare, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                   errors="replace", env=env, cwd=ROOT / workspace)
            if built.returncode:
                print(f"{package}: building what its tests need failed (cargo exit {built.returncode})")
                compile_diagnostics(built.stdout)
                return stopped("what the tests need could not be built", started, kills_before)
        expected = set()
        timing = os.environ.get("HORIZON_DATA_LEG_TIMING") == "1"
        phase = f"{package}#{number}of{count}" if number is not None else package
        if timing:
            print(f"data-leg phase={phase} compile-list-start utc={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}", flush=True)
        compile_started = time.monotonic()
        if number is not None:
            # List what exists, then skip every test another leg owns.
            listing = subprocess.run(
                [*command, "--", "--list", "--include-ignored"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, errors="replace", env=env,
                cwd=ROOT / workspace,
            )
            names = {found[1] for line in listing.stdout.splitlines() if (found := LISTED.match(line))}
            if listing.returncode or not names:
                print(f"{package}: could not list the tests to split into legs (cargo exit {listing.returncode})")
                compile_diagnostics(listing.stderr)
                return stopped("the tests could not be listed (a build or listing failure)", started, kills_before)
            others = sorted(name for name in names if not in_leg(name, number, count))
            # Every test this leg owns and does not skip must report a result.
            expected = set(names) - set(others) - set(skip)
            skip = [*skip, *others]
            print(f"{package}: leg {number} of {count} runs {len(names) - len(others)} of {len(names)} listed tests")
        skips = [arg for test in skip for arg in ("--skip", test)]
        if timing:
            # Listing already compiled the target(s). Do not forward its output.
            # This marks the existing test command, not an instruction inside a binary.
            print(f"data-leg phase={phase} test-command-start utc={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} listed={int(number is not None)} compile_list_seconds={time.monotonic() - compile_started:.3f}", flush=True)
        execution_started = time.monotonic()
        threads = min(8 if client_half else 4, int(os.environ.get("CI_CPUS", "8")))
        selection = ["--ignored", "--exact", f"--test-threads={threads}", *only[1]] if only else \
            ["--include-ignored", "--exact", f"--test-threads={threads}", *skips]
        process = subprocess.run(
            [*command, "--", *selection],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", env=env,
            cwd=ROOT / workspace,
        )
        if timing:
            print(f"data-leg phase={phase} test-command-end utc={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} execution_seconds={time.monotonic() - execution_started:.3f}", flush=True)
    results = {}
    summaries = []
    print(f"{package}: cargo exit {process.returncode}")
    long_running = dict.fromkeys(
        found[1] for line in process.stdout.splitlines() if (found := RUNNING_LONG.match(line.strip()))
    )
    if long_running:
        print(f"{package}: {len(long_running)} tests ran past 60 s: "
              + ", ".join(safe_identity(name) for name in long_running))
    target_count = None
    target_number = 0
    awaiting_count = False
    incomplete = False
    occurrences = {}
    ambiguous = set()
    max_name_length = max(map(len, expected), default=0)
    failure_diagnostics(process.stdout)
    # A target finished only if its LAST summary counts every test it started. Fixtures that rerun this test
    # executable print their own summaries first, from inside a parent test; the parent's comes after them.
    last_summary = None
    crashed = cargo_failure = False
    ended_by = None

    def stop(reason):
        return stopped(reason, started, kills_before, process, results, expected, ended_by)

    def finish_target():
        nonlocal incomplete, target_count, last_summary
        if target_count is None:
            return
        if last_summary is None or sum(map(int, SUMMARY.match(last_summary).groups())) != target_count:
            incomplete = True
        else:
            summaries.append(last_summary)
            print(f"{package}: {last_summary}")
        target_count = last_summary = None

    for line in process.stdout.splitlines():
        if TARGET.match(line):
            incomplete |= awaiting_count
            finish_target()
            target_number += 1
            awaiting_count = True
        elif awaiting_count and (found := COUNT.match(line)):
            target_count = int(found[1])
            print(f"{package}: target {target_number} started {target_count} tests")
            awaiting_count = False
        if found := RESULT.match(line):
            # A test may report more than once (below); it passes only if every record passed.
            results[found[1]] = results.get(found[1], True) and found[2] == "ok"
        elif line.startswith("screen-effect event loop error: "):
            print(line)
        elif SUMMARY.match(line):
            if target_count is not None:
                last_summary = line
        elif status := re.search(r"\((signal|exit status): (\d+)", line):
            # Emit only the numeric status, never adjacent test output.
            print(f"{package}: {status[0]})")
            # Cargo's own line for a test process that ended: 101 is libtest's "tests failed"; a signal or any
            # other status is a crash, whatever summaries came before it.
            if cargo_failure and "process didn't exit successfully" in line:
                crash = status[1] == "signal" or status[2] != "101"
                if crash and ended_by is None:
                    number = int(status[2])
                    name = signal.Signals(number).name if status[1] == "signal" and number in signal.valid_signals() else None
                    ended_by = f"{status[1]} {number}" + (f" ({name})" if name else "")
                crashed |= crash
                cargo_failure = False
        if line.startswith("error: test failed, to rerun pass "):
            # Cargo's report on a test executable; text a test prints can't be told from it otherwise.
            cargo_failure = True
        if expected:
            for name, status in listed_result_records(line, expected, max_name_length):
                if status == "ambiguous":
                    ambiguous.add(name)
                    continue
                occurrences[name] = occurrences.get(name, 0) + 1
                # --include-ignored must execute listed tests; an ignored record
                # is not promoted to a passing result or missing-test credit.
                if status != "ignored":
                    results[name] = results.get(name, True) and status == "ok"
    finish_target()
    if crashed:
        print(f"{package}: a test process crashed (cargo exit {process.returncode})")
        crash_report(package, process.stdout, expected, results)
        return stop("a test process crashed")
    # A fixture that reruns its own test executable (`run_fresh_cases`, the death-weapon cases) prints the
    # child's result record for the same name beside the parent's. Several records for one name are expected
    # and combined above, so a FAILED among them fails the test; only an unreadable record is a framing problem.
    if repeated := sum(seen > 1 for seen in occurrences.values()):
        print(f"{package}: {repeated} listed tests reported more than one result (fixtures that rerun their own "
              "test); each passes only if every result passed")
    framing_problems = {name: "ambiguous-result" for name in ambiguous}
    if framing_problems:
        remaining = _active_budget if _active_budget is not None else [FAILURE_TOTAL_BYTES]
        for name, category in sorted(framing_problems.items()):
            bounded_record(f"  {safe_identity(name)}; category={category}\n", remaining)
        print(f"{package}: {len(framing_problems)} listed identities have ambiguous result framing")
        return stop("results whose lines could not be read")
    if incomplete or awaiting_count:
        print(f"{package}: test target did not finish (cargo exit {process.returncode})")
        return stop("a test target did not finish")
    if process.returncode and not target_number:
        compile_diagnostics(process.stdout)
    if process.returncode and all(results.values()):
        print(f"{package}: cargo exited {process.returncode} without a parsed test failure")
        return stop("cargo failed without a test failure (a build failure, or a process that ended early)")
    if missing := sorted(expected - results.keys()):
        print(f"{package}: {len(missing)} listed tests of this leg never reported a result:")
        remaining = _active_budget if _active_budget is not None else [FAILURE_TOTAL_BYTES]
        shown = 0
        for name in missing:
            if remaining[0] <= 0:
                break
            framing = missing_result_framing(process.stdout, name)
            emitted, _ = bounded_record(
                f"  {safe_identity(name)}; category=missing-result; {framing}\n", remaining)
            shown += emitted
        print(f"{len(missing) - shown} missing identities not shown (shared total byte budget)")
        return stop("listed tests never reported a result")
    if not summaries:
        # Unknown output is not necessarily a compiler diagnostic: never forward
        # a malformed test process's captured payload to public CI logs.
        print(f"{package}: no completed test-target summary; raw output withheld")
        return stop("no test target finished")
    return results


def main(shards, report=None, without_data=False):
    global _active_budget
    verdict = {"shards": list(shards), "problems": [], "passed": [], "incomplete": []}

    def problem(package, test, kind, policy=None):
        verdict["problems"].append({"package": package, "test": safe_identity(test), "kind": kind,
                                    "panic": _panic_locations.get(test), "policy": policy})
    known = known_failures()
    inputs = harness_inputs()
    files = {(row.package, row.test): row.file for row in listed_entries()}
    skipped = {"harness", "manual"} if has_vulkan() and not without_data else {"harness", "manual", "gpu"}
    if "gpu" in skipped and not without_data:
        print("no Vulkan driver on this runner: the tests marked `gpu` are skipped")
    problems = []
    accepted_unshown = 0
    # Reuse the hash selection in two consecutive processes on the same runner.
    # Resource-backed client-lib families retain registries across tests; releasing
    # the process between halves bounds that retention without another CI leg.
    daily = daily_rows()
    # Validate the catalogue before installing a shared budget: a rejected policy
    # must not leave diagnostic state behind for another invocation.
    _active_budget = [FAILURE_TOTAL_BYTES]
    _panic_locations.clear()
    # With --without-data: every test that needs the vanilla data, as (package, test); none otherwise.
    left_out, left_out_count = set(), 0
    if without_data:
        left_out = ({key for key, mark in known.items() if mark in DATA_MARKS} | set(inputs)
                    | {(row.package, row.test) for row in daily if row.reason in DATA_REASONS})
        # The daily-rows legs' own rows that go with it (a package shard counts its tests below).
        if set(shards) & set(ignored_shards.DAILY_ROWS_LEGS.values()):
            left_out_count = sum(1 for row in daily if (row.package, row.test) in left_out
                                 and not shard_runs(row.package, row.reference)
                                 and ignored_shards.daily_rows_leg(row.package) in shards)
        daily = [row for row in daily if (row.package, row.test) not in left_out]
        known = {key: mark for key, mark in known.items() if key not in left_out}
        inputs = {}
    groups = daily_rows_groups(daily)
    selections = [
        selection
        for shard in shards
        for selection in (
            ("horizon-client-lib#1of2", "horizon-client-lib#2of2")
            if shard == "horizon-client-lib" else
            [f"{shard}:{package}:{target}" for package, target in groups
             if ignored_shards.daily_rows_leg(package) == shard]
            if shard in ignored_shards.DAILY_ROWS_LEGS.values() else (shard,)
        )
    ]
    for shard in selections:
        # Written before the run, so a job stopped meanwhile (its time limit, a cancel) still uploads a report that
        # names the run in progress; the run's own outcome replaces it.
        write_report(report, dict(verdict, incomplete=[*verdict["incomplete"], {
            "shard": shard, "in_progress": True, "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}]))
        _stopped.clear()
        daily_group = shard.split(":", 1)[0] in ignored_shards.DAILY_ROWS_LEGS.values()
        if daily_group:
            _, package, target = shard.split(":", 2)
            print(f"{package} {target}: {len(groups[package, target])} daily tests", flush=True)
            results = run(package, [], only=(target, groups[package, target]))
            expected_daily = groups[package, target]
        else:
            package, number, count = parse_shard(shard)
            expected_daily = [row.test for row in daily if row.package == package
                              and shard_runs(package, row.reference) and in_leg(row.test, number, count)]
            skip = [test for (listed, test), mark in known.items() if listed == package and mark in skipped]
            out = sorted(test for listed, test in left_out if listed == package and in_leg(test, number, count))
            skip += out
            left_out_count += len(out)
            for (listed, test), variable in sorted(inputs.items()):
                if listed == package and in_leg(test, number, count) and not os.environ.get(variable):
                    print(f"{package} {safe_identity(test)}: refused, needs {variable}; run by the parity harness")
                    skip.append(test)
            results = run(package, skip, number, count) if number is not None else run(package, skip)
        if results is None:
            print(f"{shard}: did not complete: " + json.dumps(_stopped, sort_keys=True), flush=True)
            problems.append(f"{shard}: the test run did not complete")
            verdict["incomplete"].append({"shard": shard, **_stopped})
            continue
        for test, passed in sorted(results.items()):
            listed = (package, test) in known
            policy_test = test
            test = safe_identity(test)
            if not passed and listed and known[(package, policy_test)] != "gpu":
                emitted, _ = bounded_record(
                    f"{package} {test}: accepted failure policy={files.get((package, policy_test), POLICY_PATH)} "
                    f"mark={known[(package, policy_test)] or 'known'}\n", _active_budget)
                accepted_unshown += not emitted
            policy = files.get((package, policy_test), POLICY_PATH) if listed else None
            if not passed and not listed:
                problems.append(f"{package} {test}: failed, and isn't a known failure")
                problem(package, policy_test, "failed")
            elif not passed and known[(package, policy_test)] == "gpu":
                problems.append(f"{package} {test}: failed with a Vulkan driver present")
                problem(package, policy_test, "gpu", policy)
            elif passed and listed and known[(package, policy_test)] is None:
                problems.append(f"{package} {test}: passes now; take it off {policy}")
                problem(package, policy_test, "passes-now", policy)
            elif passed:
                verdict["passed"].append({"package": package, "test": test})
        for test in expected_daily:
            if test not in results:
                problems.append(f"{package} {safe_identity(test)}: listed in {files[package, test]} but no longer runs")
                problem(package, test, "no-longer-runs", files[package, test])
        if daily_group:
            continue
        for (listed, test), mark in known.items():
            if listed == package and mark not in skipped and in_leg(test, number, count) and test not in results:
                problems.append(f"{package} {safe_identity(test)}: listed in {files.get((package, test), POLICY_PATH)} but no longer runs")
                problem(package, test, "no-longer-runs", files.get((package, test), POLICY_PATH))
    if without_data:
        print(f"--without-data: {left_out_count} tests left out, they need the vanilla data "
              f"(daily rows with reason {' or '.join(DATA_REASONS)}, and {', '.join(DATA_MARKS)}, needs and needs-plain rows)")
    shown = 0
    for problem in problems:
        emitted, _ = bounded_record(f"::error::{problem}\n", _active_budget)
        shown += emitted
    if problems:
        print(f"policy diagnostics: {len(problems) - shown} problems not shown (shared total byte budget)")
    if accepted_unshown:
        print(f"{accepted_unshown} accepted policy records not shown (shared total byte budget)")
    _active_budget = None
    write_report(report, verdict)
    return 1 if problems else 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("shards", nargs="*", metavar="SHARD")
    parser.add_argument("--needs", metavar="VARIABLE", help="the tests marked `needs VARIABLE`")
    each = parser.add_mutually_exclusive_group()
    each.add_argument("--one-process-each", action="store_true", help="run them, each alone in its own process")
    each.add_argument("--list", action="store_true", help="print their commands")
    parser.add_argument("--jobs", type=int, help="cargo's -j for --needs")
    parser.add_argument("--report", metavar="FILE", help="also write the verdict as JSON for the daily summary")
    parser.add_argument("--without-data", action="store_true",
                        help="leave out the tests that need the prepared vanilla data (a runner that has none)")
    parser.add_argument("--daily-tests", nargs=2, metavar=("PACKAGE", "TARGET"),
                        help="print the daily rows' tests of one package's target, one per line")
    args = parser.parse_args()
    if args.daily_tests:
        # Nothing at all for no rows: an empty line would read as one empty name.
        sys.stdout.write("".join(f"{test}\n" for test in daily_tests(*args.daily_tests)))
    elif args.needs and args.list:
        print("\n".join(shlex.join(command) for command in needs_commands(args.needs, args.jobs)))
    elif args.needs and args.one_process_each:
        sys.exit(run_one_process_each(args.needs, args.jobs, args.report))
    elif args.needs or args.one_process_each or args.list:
        parser.error("--needs takes --one-process-each or --list, and they take --needs")
    else:
        sys.exit(main(args.shards, args.report, args.without_data))
