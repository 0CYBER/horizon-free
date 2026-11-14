"""Run a workspace's built test binaries several at a time instead of one by one.

    python3 tools/ci/run_test_binaries.py --jobs 3 -- cargo test $JOBS --profile test --workspace --lib --bins --tests

The cargo command (without anything after its own `--`) is run once more with `--no-run
--message-format=json-render-diagnostics` to list the test executables, already built, so nothing compiles. Each
binary runs from its package's directory with the arguments after `--`, the package's main CARGO_* variables
(manifest dir and path, package name and version, crate name), CARGO, and a library path with the target's and the
toolchain's libraries; this is close to `cargo test`'s environment, not all of it. Output goes to a temporary file per
binary and is printed whole when the binary ends. Binaries start largest first, since the largest are the slow ones.
Every binary runs; one that fails, crashes, is killed or cannot start fails the run, and the run ends with a list of
the failing binaries and their failed tests' names. A listing with no test binaries fails too.

`--network-guard LIBRARY` (Linux verify) preloads each binary and records inet and resolver calls. Green
binaries run normally, with the same timing logs and output. Only a binary recording a violation is rerun one
test at a time to name the culprits, even when they swallow the refusal. A binary still fails if isolation cannot
reproduce its violation. Missing preload fails closed by scheduled PID. Unix sockets remain allowed.

`--time-limits WARN,FAIL` (verify uses `2,5`) is the unit tier's timing guard (CONTRIBUTING.md "CI tiers"): every
binary also gets libtest's `-Z unstable-options --report-time --logfile FILE`. libtest then adds each test's time to
its result line and writes one record per test (`ok a::b <0.012s>`) to FILE, from its own thread, apart from the
output that tests and their child processes interleave on stdout; the same tests run and fail the same way. Those
flags need RUSTC_BOOTSTRAP=1, set in the test processes only (never for the build), so a test's own child processes
inherit it. The guard fails closed: each binary is also listed (`--list`), and a binary that passed but whose log
lacks a timed record for one of its listed, not ignored tests fails the run, naming the binary and the tests.
A test over FAIL seconds is rerun once, alone after all binary workers finish, with the same binary and environment.
Only a second time over FAIL fails the timing guard; a failed or unmeasurable rerun also fails closed. Both times
are printed. One over WARN only warns. The time is libtest's elapsed (wall) time with `--jobs` binaries and each binary's test threads running at
once, not an isolated measurement, so FAIL keeps a margin over the 2 s target. A `harness = false` target runs its own main, not libtest: it runs, untimed. Only these binaries are timed: the
documentation examples and the Python steps of `verify` are not under the guard. The run ends with the slowest five
tests and the summed test seconds. (libtest's own `--ensure-time` was the alternative: it fails a slow test inside
the binary with libtest's words, not the instruction, and needs a threshold variable per kind of test target.)
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import network_guard


def package_identity(package_id: str) -> tuple[str, str]:
    """(name, version) from a Cargo package id, old (`name 1.0 (path+...)`) or new (`path+...#name@1.0`)."""
    if found := re.fullmatch(r"(\S+) (\S+) \(.*\)", package_id):
        return found[1], found[2]
    fragment = package_id.rsplit("#", 1)[-1]
    name, _, version = fragment.rpartition("@")
    if not name:  # `path+file:///x/horizon-world#0.1.0`: the name is the last path segment.
        name = package_id.split("#", 1)[0].rstrip("/").rsplit("/", 1)[-1]
    return name, version


def test_binaries(messages: str) -> list[dict]:
    """The test executables Cargo reported, each once, with what running it needs."""
    binaries, seen = [], set()
    for line in messages.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or not message["profile"]["test"]:
            continue
        executable = message.get("executable")
        if not executable or executable in seen:
            continue
        seen.add(executable)
        name, version = package_identity(message["package_id"])
        manifest = message["manifest_path"]
        binaries.append({
            "executable": executable,
            "label": f"{name} {Path(message['target']['src_path']).name} ({Path(executable).name})",
            "cwd": str(Path(manifest).parent),
            "env": {
                "CARGO_MANIFEST_DIR": str(Path(manifest).parent),
                "CARGO_MANIFEST_PATH": manifest,
                "CARGO_PKG_NAME": name,
                "CARGO_PKG_VERSION": version,
                "CARGO_CRATE_NAME": message["target"]["name"].replace("-", "_"),
            },
            "libtest": uses_libtest(manifest, message["target"]),
        })
    return binaries


def uses_libtest(manifest: str, target: dict) -> bool:
    """False for a target its manifest builds with `harness = false`: its own main, not libtest, so no list or log."""
    try:
        table = tomllib.loads(Path(manifest).read_text())
    except (OSError, tomllib.TOMLDecodeError):
        return True
    sections = {"test": "test", "bin": "bin", "bench": "bench", "example": "example"}
    entries = [entry for kind in target.get("kind", []) for entry in table.get(sections.get(kind, ""), [])
               if entry.get("name") == target["name"]]
    if "lib" in target.get("kind", []) or "rlib" in target.get("kind", []):
        entries.append(table.get("lib", {}))
    return all(entry.get("harness", True) for entry in entries)


def listed_tests(binary: dict, env: dict, test_args: list[str]) -> list[str] | None:
    """The tests `binary` holds that these arguments select (`--list`), or None if it cannot list them."""
    try:
        listing = subprocess.run([binary["executable"], *test_args, "--list"], cwd=binary["cwd"], env=env,
                                 capture_output=True, text=True, errors="replace")
    except OSError:
        return None
    if listing.returncode:
        return None
    names = [found[1] for line in listing.stdout.splitlines() if (found := re.match(r"^(\S+): test$", line))]
    # The listing must be the plain one, ending in its own count: any other form (`--format json`, an output that
    # interleaved) is unreadable here, never an empty suite.
    count = re.search(r"^(\d+) tests?, \d+ benchmarks?$", listing.stdout, re.M)
    return names if count and int(count[1]) == len(names) else None


def read_log(text: str) -> dict[str, tuple[str, float | None]]:
    """libtest's --logfile records: test name -> (its status, its time or None).

    A record is the status word, an ignored test's message if it has one, the name (never containing a space, so the
    last word) and the time: `ok a::b <0.012s>`, `failed a::c <1.000s>`, `ignored a::d`,
    `ignored: daily a::e`, `ignored: measurement: cargo test ... a::f`."""
    records = {}
    for line in text.splitlines():
        timed = re.search(r" <(\d+(?:\.\d+)?)s>$", line)
        words = (line[:timed.start()] if timed else line).split()
        if len(words) >= 2 and (status := re.match(r"^\w+", words[0])):
            records[words[-1]] = (status[0], float(timed[1]) if timed else None)
    return records


def run_one(binary: dict, extra_env: dict, test_args: list[str], timed: bool = False, *, listing_env: dict | None = None) -> dict:
    """Run one test binary with its output in a temporary file, never raising: a binary that cannot start fails.

    `timed`: also list its tests and write libtest's per-test log, for the timing guard (`listed`, `log`)."""
    result = {"binary": binary, "output": tempfile.TemporaryFile("w+", errors="replace"), "max_rss_kib": 0}
    env = {**os.environ, **extra_env, **binary["env"]}
    log = None
    timed = timed and binary.get("libtest", True)
    if timed:
        list_env = env if listing_env is None else {**os.environ, **listing_env, **binary["env"]}
        result["listed"] = [binary["isolated_test"]] if "isolated_test" in binary else listed_tests(binary, list_env, test_args)
        handle, log = tempfile.mkstemp(prefix="libtest-", suffix=".log")
        os.close(handle)
        test_args = [*test_args, "--logfile", log]
    started = time.monotonic()
    try:
        process = subprocess.Popen([binary["executable"], *test_args], cwd=binary["cwd"],
                                   env=env, stdout=result["output"], stderr=subprocess.STDOUT)
    except OSError as error:
        result.update(status=f"could not start ({error.strerror or error})", seconds=0.0)
        if log:
            os.unlink(log)
        return result
    result["pid"] = process.pid
    _, wait_status, usage = os.wait4(process.pid, 0)
    process.returncode = code = os.waitstatus_to_exitcode(wait_status)
    status = "ok" if code == 0 else f"killed by signal {-code}" if code < 0 else f"exit {code}"
    result.update(status=status, seconds=time.monotonic() - started, max_rss_kib=usage.ru_maxrss)
    if log:
        result["log"] = read_log(Path(log).read_text(errors="replace"))
        os.unlink(log)
    return result


def untimed(result: dict) -> list[str]:
    """The listed tests a passing binary ran without a timed record in its log (the guard fails closed on them).

    An ignored test has a record without a time and is not expected to have one; a binary that failed is already
    a failure, and its missing records are not reported again."""
    if result["status"] != "ok" or "listed" not in result:
        return []
    if result.get("listed") is None:
        return ["(its tests could not be listed)"]
    log = result.get("log", {})
    return [name for name in result["listed"]
            if name not in log or (log[name][0] != "ignored" and log[name][1] is None)]


# A libtest result line, with the time --report-time adds.
# A failed result line, with the time --report-time adds.
FAILED = re.compile(r"^test (\S+)(?: - should panic)? \.\.\. FAILED(?: <\d+(?:\.\d+)?s>)?$")


def report(result: dict) -> list[str]:
    """Print one finished binary's output, streamed from its file, and return its failed test names."""
    print(f"     Running {result['binary']['label']} [{result['seconds']:.1f}s, "
          f"max RSS {result['max_rss_kib'] // 1024} MiB]")
    output, failed_tests = result["output"], []
    output.seek(0)
    for line in output:
        print(line, end="")
        if found := FAILED.match(line.rstrip("\n")):
            failed_tests.append(found[1])
    output.close()
    print(flush=True)
    return failed_tests


def timing_verdict(times: list[tuple[float, str, str]], warn: float, fail: float) -> tuple[list[str], list[str]]:
    """(errors, warnings) for tests over `fail` and over `warn` seconds; `times` holds (seconds, test, binary)."""
    errors, warnings = [], []
    for seconds, test, label in sorted(times, reverse=True):
        if seconds > fail:
            errors.append(f"{label}: {test} took {seconds:.2f}s, over the unit tier's {fail:g} s limit: mark it "
                          '#[ignore = "daily"] and add its row (tools/ci/move_to_daily.py, '
                          "tools/ci/ignored-tests/README.md), or make it faster")
        elif seconds > warn:
            warnings.append(f"{label}: {test} took {seconds:.2f}s, over the unit tier's {warn:g} s target")
    return errors, warnings


def confirm_slow_tests(times: list[tuple[float, str, str]], binaries: list[dict], extra_env: dict,
                       test_args: list[str], fail: float, guard: Path | None = None) -> list[str]:
    """Retry only hard-limit candidates, serially after the pool drains; never forgive a functional failure.
    With `guard` the rerun is preloaded with the network guard like the first run."""
    errors = []
    by_label = {binary["label"]: binary for binary in binaries}
    for first, test, label in sorted(times, reverse=True):
        if first <= fail:
            continue
        # Replace filters/thread counts rather than append: libtest ORs positional filters. Keep ignored selection
        # when this runner is used outside verify, but run exactly this test on one libtest thread.
        selection = [arg for arg in test_args if arg in ("--ignored", "--include-ignored")]
        args = ["-Z", "unstable-options", "--report-time", *selection, "--exact", test, "--test-threads=1"]
        result = (network_guard.run((by_label[label], args), extra_env, True, guard, run_one) if guard
                  else run_one(by_label[label], extra_env, args, timed=True))
        report(result)
        status, seconds = result.get("log", {}).get(test, (None, None))
        measured = f"{seconds:.2f}s" if seconds is not None else "unavailable"
        print(f"timing confirmation: {label}: {test}: first {first:.2f}s, isolated {measured}", flush=True)
        if result["status"] != "ok" or status != "ok" or seconds is None:
            errors.append(f"{label}: {test}: timing rerun failed or has no passing timed record "
                          f"({result['status']}); first {first:.2f}s, isolated {measured}")
        elif seconds > fail:
            failures, _ = timing_verdict([(seconds, test, label)], fail, fail)
            errors.append(f"{failures[0]} (first {first:.2f}s, isolated {seconds:.2f}s)")
    return errors


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--network-guard", type=Path, help="Linux preload library; fail recorded inet and resolver calls, rerunning only a violating binary test by test")
    parser.add_argument("--jobs", type=int, required=True, help="test binaries running at once")
    parser.add_argument("--time-limits", metavar="WARN,FAIL", help="seconds: warn about a test over WARN, fail one "
                        "over FAIL (adds libtest's --report-time)")
    parser.add_argument("cargo", nargs=argparse.REMAINDER,
                        help="the cargo test command, after --; arguments after its own -- go to every test binary")
    args = parser.parse_args(argv)
    command = args.cargo[1:] if args.cargo[:1] == ["--"] else args.cargo
    command, test_args = (command[:command.index("--")], command[command.index("--") + 1:]) if "--" in command \
        else (command, [])
    limits = None
    if args.time_limits:
        warn, _, fail = args.time_limits.partition(",")
        limits = float(warn), float(fail)
        test_args = ["-Z", "unstable-options", "--report-time", *test_args]
    listing = subprocess.run([*command, "--no-run", "--message-format=json-render-diagnostics"],
                             stdout=subprocess.PIPE, text=True)
    if listing.returncode:
        print(f"::error::listing the test binaries failed (cargo exit {listing.returncode})")
        return listing.returncode
    binaries = sorted(test_binaries(listing.stdout), key=lambda b: os.path.getsize(b["executable"]) if os.path.exists(b["executable"]) else 0,
                      reverse=True)
    if not binaries:
        print("::error::cargo listed no test binaries: nothing would be tested")
        return 1
    # The toolchain's std and the target's dynamic libraries on the library path (a proc-macro crate's test binary
    # links std dynamically), and CARGO set to the toolchain's cargo, as `cargo test` sets them.
    toolchain = [command[1]] if len(command) > 1 and command[1].startswith("+") else []
    rustc = Path(shutil.which(command[0]) or "").with_name("rustc")
    rustc = str(rustc) if rustc.is_file() else shutil.which("rustc")
    if rustc is None:
        print("::error::no rustc beside cargo or on PATH: the toolchain's std cannot be put on the library path")
        return 1
    sysroot = subprocess.run([rustc, *toolchain, "--print", "sysroot"], capture_output=True, text=True,
                             check=True).stdout.strip()
    host = re.search(r"^host: (\S+)$", subprocess.run([rustc, *toolchain, "-vV"], capture_output=True, text=True,
                                                     check=True).stdout, re.M)[1]
    deps = {str(Path(b["executable"]).parent) for b in binaries}
    library_path = os.pathsep.join([*sorted(deps), f"{sysroot}/lib/rustlib/{host}/lib", f"{sysroot}/lib",
                                    *filter(None, [os.environ.get("LD_LIBRARY_PATH")])])
    toolchain_cargo = Path(sysroot, "bin", "cargo")
    print(f"library path: {library_path}", flush=True)
    extra_env = {"LD_LIBRARY_PATH": library_path,
                 "CARGO": str(toolchain_cargo) if toolchain_cargo.is_file() else os.environ.get("CARGO", "cargo")}
    if limits:
        extra_env["RUSTC_BOOTSTRAP"] = "1"
    print(f"running {len(binaries)} test binaries, {args.jobs} at a time", flush=True)
    failures, peaks, times, missing, started = [], [], [], [], time.monotonic()
    def results():
        with ThreadPoolExecutor(args.jobs) as pool:
            if not args.network_guard:
                yield from pool.map(lambda b: run_one(b, extra_env, test_args, timed=bool(limits)), binaries)
                return
            yield from pool.map(lambda b: network_guard.run_binary(
                b, extra_env, test_args, bool(limits), args.network_guard.resolve(), run_one, listed_tests), binaries)
    for result in results():
        failed_tests = report(result)
        failed_tests.extend(name for name in result.get("network_tests", []) if name not in failed_tests)
        times.extend((seconds, test, result["binary"]["label"])
                     for test, (_, seconds) in result.get("log", {}).items() if seconds is not None)
        if limits and (lost := untimed(result)):
            missing.append((result["binary"]["label"], lost))
        peaks.append((result["max_rss_kib"], result["binary"]["label"]))
        if result["status"] != "ok":
            failures.append((result["binary"]["label"], result["status"], failed_tests))
    print(f"ran {len(binaries)} test binaries in {time.monotonic() - started:.1f}s; {len(failures)} failed")
    print("largest resident sets: " + ", ".join(f"{label} {kib // 1024} MiB" for kib, label in sorted(peaks)[-5:][::-1]))
    slow = []
    if limits:
        _, warnings = timing_verdict(times, *limits)
        slow = confirm_slow_tests(times, binaries, extra_env, test_args, limits[1],
                                  args.network_guard.resolve() if args.network_guard else None)
        print(f"{len(times)} tests timed, {sum(seconds for seconds, _, _ in times):.1f} test-seconds in total; "
              "slowest: " + ", ".join(f"{test} {seconds:.2f}s" for seconds, test, _ in sorted(times)[-5:][::-1]))
        for warning in warnings:
            print(f"::warning::{warning}")
        for error in slow:
            print(f"::error::{error}")
        for label, lost in missing:
            print(f"::error::{label}: {len(lost)} listed tests have no timed record in libtest's log, so the guard "
                  f"cannot judge them: {', '.join(lost[:5])}{' ...' if len(lost) > 5 else ''}")
            slow.append(label)
    if failures:
        # At the end, where a log tail shows it: every failing binary and its failed tests' names.
        print("FAILED TESTS")
        for label, status, failed_tests in failures:
            print(f"::error::{label}: {status}")
            for name in failed_tests:
                print(f"    {name}")
    return 1 if failures or slow else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
