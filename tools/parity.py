#!/usr/bin/env python3
"""Run the vanilla differential harnesses (tools/test_*_parity.py) quickly.

    python3 tools/parity.py                 # harnesses whose inputs changed, or that never passed
    python3 tools/parity.py --all           # every harness, from scratch (the audit)
    python3 tools/parity.py --fresh ...     # don't reuse vanilla oracle results
    python3 tools/parity.py food light      # these harnesses (names without test_/_parity.py)
    python3 tools/parity.py --failed        # the ones that failed last time
    python3 tools/parity.py --list          # what would run, without running it
    python3 tools/parity.py --status        # last result of every harness

Each harness runs unmodified, in parallel, under a tracer that records what it
read: its Python modules, Java oracle sources, other repository files, and the
Rust sources of every probe it executed (from Cargo's dep-info). A harness
reruns only when one of those files, the pinned vanilla artifacts or the
toolchains change; otherwise its last pass stands.

Vanilla oracles are reused too: tools/parity_shim.py stands in for java and
javac and replays an oracle run whose inputs are unchanged, so a Rust-only
change costs only the Rust side. A harness that fails while reusing oracle
results is rerun fresh, and stops reusing them if it then passes. --all and
--fresh never reuse them.

Harness probes build into .local/parity-target with release optimisations but
without LTO and with 16 codegen units (`PROFILE` below), which builds in
seconds instead of minutes. Rust does not let those settings change results:
there is no fast-math, and overflow checks stay as in release. Probes built in
the dev profile are optimised too, keeping their overflow checks. Probes the
recorded harnesses need are built in one Cargo run before the harnesses start,
so parallel harnesses don't queue on Cargo's lock.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
LOCAL = ROOT / ".local"
STATE_DIR = LOCAL / "parity-runner"
STATE = STATE_DIR / "state.json"
TARGET = LOCAL / "parity-target"
PROFILE = {
    "CARGO_PROFILE_RELEASE_LTO": "false",
    "CARGO_PROFILE_RELEASE_CODEGEN_UNITS": "16",
    "CARGO_PROFILE_RELEASE_INCREMENTAL": "true",
    # Probes built in the dev profile keep its overflow checks and debug
    # assertions, but run optimised.
    "CARGO_PROFILE_DEV_OPT_LEVEL": "3",
}
SHIM = STATE_DIR / "shim"
# Results shared by every worktree using the same vanilla data (.local/minecraft
# links to a shared directory in parallel worktrees): passes keyed by content,
# so a harness whose inputs match one that passed elsewhere doesn't run again.
SHARED = Path(os.environ.get("PARITY_SHARED") or (LOCAL / "minecraft").resolve().parent / "parity-shared")
MACHINE_JOBS = int(os.environ.get("PARITY_MACHINE_JOBS") or max(1, (os.cpu_count() or 4) // 2))
DEFAULT_MEMORY = 3 << 30  # assumed peak for a harness that never ran here


def harnesses():
    return {path.name[len("test_"):-len("_parity.py")]: path for path in sorted((ROOT / "tools").glob("test_*_parity.py"))}


def load_state():
    return json.loads(STATE.read_text()) if STATE.exists() else {"harnesses": {}, "hashes": {}}


def save_state(state):
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    temporary = STATE.with_suffix(".tmp")
    temporary.write_text(json.dumps(state, indent=1, sort_keys=True))
    temporary.replace(STATE)


class Hasher:
    """Content hashes, reused while a file's size and mtime stay the same."""

    def __init__(self, cache):
        self.cache = cache

    def __call__(self, path):
        try:
            stat = os.stat(path)
        except FileNotFoundError:
            return "missing"
        stamp = [stat.st_size, stat.st_mtime_ns]
        cached = self.cache.get(str(path))
        if cached and cached[0] == stamp:
            return cached[1]
        with open(path, "rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        self.cache[str(path)] = [stamp, digest]
        return digest


def environment_key(hasher):
    """Everything outside the recorded dependencies that can change a result."""
    def version(command):
        try:
            result = subprocess.run(command, capture_output=True, text=True, cwd=ROOT)
            return (result.stdout + result.stderr).strip()
        except FileNotFoundError:
            return "missing"
    lock = json.loads((ROOT / "research-lock.json").read_text())
    version_dir = LOCAL / "minecraft" / lock["minecraft"]["version"]
    parts = [version(["rustc", "-V"]), version(["java", "-version"]), version(["javac", "-version"]),
             json.dumps(PROFILE, sort_keys=True), hasher(ROOT / "research-lock.json"), hasher(ROOT / "Cargo.lock"),
             hasher(version_dir / "prepared.json"), hasher(version_dir / "manifest.json")]
    return hashlib.sha256("\n".join(parts).encode()).hexdigest()


def key(record, hasher, environment):
    digest = hashlib.sha256(environment.encode())
    for path in sorted(record["dependencies"]):
        digest.update(f"{path}\0{hasher(ROOT / path)}\n".encode())
    return digest.hexdigest()


def inside(path, root):
    try:
        Path(path).resolve().relative_to(root)
        return True
    except (ValueError, OSError):
        return False


def dep_info(binary):
    """Source files Cargo recorded for a built binary (its .d file, spaces escaped as "\\ ")."""
    info = Path(str(binary) + ".d")
    if not info.exists():
        return []
    sources = []
    for line in info.read_text().splitlines():
        _, separator, rest = line.partition(": ")
        if separator:
            sources += [part.replace("\\ ", " ") for part in re.split(r"(?<!\\) ", rest.strip()) if part]
    return sources


def artifact_path(kind, name, profile):
    directory = "debug" if profile == "dev" else profile
    return TARGET / directory / ("examples" if kind == "example" else "") / name


def artifacts_of(arguments):
    """The [kind, name, profile] targets a cargo build/run command line names."""
    arguments = arguments[:arguments.index("--")] if "--" in arguments else arguments
    profile = "release" if "--release" in arguments else "dev"
    if "--profile" in arguments:
        profile = arguments[arguments.index("--profile") + 1]
    found = []
    for flag, kind in (("--example", "example"), ("--bin", "bin")):
        for index, argument in enumerate(arguments[:-1]):
            if argument == flag:
                found.append([kind, arguments[index + 1], profile])
    return found


def dependencies(trace):
    """Repository files a traced harness depended on, relative to ROOT."""
    found = set(trace["modules"])
    found.update(trace["read"])
    artifacts = []
    for arguments in trace["commands"]:
        program = Path(arguments[0]).name if arguments else ""
        if program == "cargo" and ("build" in arguments or "run" in arguments):
            artifacts += artifacts_of(arguments)
        if arguments and inside(arguments[0], TARGET):
            found.update(dep_info(Path(arguments[0])))
        if program == "javac":
            found.update(java_sources(arguments))
        for argument in arguments[1:]:
            if isinstance(argument, str) and os.path.isfile(argument):
                found.add(argument)
    for kind, name, profile in artifacts:
        found.update(dep_info(artifact_path(kind, name, profile)))
    kept = set()
    for path in found:
        path = Path(path) if os.path.isabs(path) else ROOT / path
        if inside(path, ROOT) and not inside(path, LOCAL) and not inside(path, ROOT / "target") and path.is_file():
            kept.add(str(path.resolve().relative_to(ROOT)))
    unique = []
    for artifact in artifacts:
        if artifact not in unique:
            unique.append(artifact)
    return sorted(kept), unique


def java_sources(arguments):
    """Java files javac compiled: the named ones, plus those it pulled from -sourcepath into -d."""
    sources = [argument for argument in arguments if str(argument).endswith(".java")]
    if "-d" in arguments and "-sourcepath" in arguments:
        output = Path(arguments[arguments.index("-d") + 1])
        source_roots = arguments[arguments.index("-sourcepath") + 1].split(os.pathsep)
        for compiled in output.rglob("*.class") if output.is_dir() else []:
            relative = compiled.relative_to(output).with_suffix("")
            top = relative.with_name(relative.name.split("$")[0] + ".java")
            for root in source_roots:
                if (Path(root) / top).is_file():
                    sources.append(str(Path(root) / top))
    return sources


def run_environment(reuse_oracles=False):
    environment = dict(os.environ, CARGO_TARGET_DIR=str(TARGET), PYTHONDONTWRITEBYTECODE="1", HORIZON_NO_AUDIO="1", **PROFILE)
    environment.pop("CARGO_BUILD_JOBS", None)
    if reuse_oracles:
        # tools/parity_shim.py stands in for java and javac and replays unchanged oracle runs.
        SHIM.mkdir(parents=True, exist_ok=True)
        for name in ("java", "javac"):
            link = SHIM / name
            if not link.is_symlink():
                link.symlink_to(ROOT / "tools" / "parity_shim.py")
        environment.update(PARITY_SHIM_REAL_PATH=os.environ.get("PATH", ""), PARITY_SHIM_CACHE=str(STATE_DIR / "oracle-cache"),
                           PATH=f"{SHIM}{os.pathsep}{os.environ.get('PATH', '')}")
    return environment


def prebuild(artifacts, jobs):
    """Build every recorded probe in one Cargo run so harnesses don't serialise on its lock."""
    environment = run_environment()
    environment["CARGO_BUILD_JOBS"] = str(jobs)
    errors = ""
    for profile in sorted({profile for _, _, profile in artifacts}):
        command = ["cargo", "build", "--locked", "--workspace", "--profile", profile]
        for kind, name, _ in (artifact for artifact in artifacts if artifact[2] == profile):
            command += ["--examples", "--bins"] if kind == "all" else [f"--{kind}", name]
        result = subprocess.run(command, cwd=ROOT, env=environment, capture_output=True, text=True)
        if result.returncode:
            errors += result.stderr
    return not errors, errors


LARGE_OUTPUT = 64 << 20
OWN_DIRECTORIES = {"parity-target", "parity-runner", "rea", "logs", "gate"}


def outputs_listing(name, fresh=False):
    path = STATE_DIR / "oracle-outputs" / f"{name}.txt"
    path.parent.mkdir(parents=True, exist_ok=True)
    if fresh:
        path.write_text("")
    return path


def clear_large_outputs(name, trace):
    """After a pass, delete the large files a harness regenerates on every run.

    Harness outputs reach gigabytes and used to stay in every worktree. Vanilla
    oracle outputs are kept, since reusing an oracle run needs them in place;
    everything else large under the harness's .local directories is rebuilt on
    its next run."""
    local = LOCAL.resolve()
    keep = {os.path.realpath(line) for line in outputs_listing(name).read_text().splitlines() if line}
    directories = set()
    for command in trace["commands"]:
        for argument in command[1:]:
            path = Path(os.path.realpath(argument if os.path.isabs(argument) else ROOT / argument))
            try:
                top = path.relative_to(local).parts[0]
            except (ValueError, IndexError):
                continue
            if top not in OWN_DIRECTORIES:
                directories.add(local / top)
    for directory in directories:
        for file in (directory.rglob("*") if directory.is_dir() else []):
            try:
                if file.is_file() and not file.is_symlink() and file.stat().st_size >= LARGE_OUTPUT and str(file) not in keep:
                    file.unlink()
            except OSError:
                pass


def publish(name, record):
    """Share a pass, so any worktree with the same inputs can reuse it."""
    if record["status"] != "pass" or "key" not in record:
        return
    for path, data in ((SHARED / "passes" / name / f"{record['key']}.json", record),
                       (SHARED / "dependencies" / f"{name}.json", {k: record[k] for k in ("dependencies", "artifacts")})):
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name(f"{path.name}.{os.getpid()}.tmp")
        temporary.write_text(json.dumps(data))
        temporary.replace(path)


def machine_slot():
    """One of MACHINE_JOBS lock files shared by every runner on this machine, or None if all are taken."""
    import fcntl
    slots = SHARED / "slots"
    slots.mkdir(parents=True, exist_ok=True)
    for index in range(MACHINE_JOBS):
        handle = open(slots / f"{index}.lock", "w")
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return handle
        except OSError:
            handle.close()
    return None


def memory_available():
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) * 1024
    return 16 << 30


def run(selected, state, hasher, environment_digest, jobs, timeout, memory_budget, reuse_oracles):
    logs = STATE_DIR / "logs"
    traces = STATE_DIR / "traces"
    logs.mkdir(parents=True, exist_ok=True)
    traces.mkdir(parents=True, exist_ok=True)
    records = state["harnesses"]
    # Longest first, so the slow harnesses don't start last.
    queue = sorted(selected, key=lambda name: -records.get(name, {}).get("seconds", 1e9))
    running = {}
    results = {}
    retried = set()
    started_all = time.monotonic()
    while queue or running:
        in_use = sum(item[3] for item in running.values())
        for name in list(queue):
            if len(running) >= jobs:
                break
            expected = records.get(name, {}).get("peak_bytes", DEFAULT_MEMORY)
            if running and in_use + expected > memory_budget:
                continue
            slot = machine_slot()
            if slot is None:
                break  # other runners on this machine hold every slot
            queue.remove(name)
            reuse = reuse_oracles and records.get(name, {}).get("oracle_reuse", True) and name not in retried
            trace = traces / f"{name}.json"
            trace.unlink(missing_ok=True)
            log = (logs / f"{name}.log").open("w")
            environment = run_environment(reuse)
            environment["PARITY_SHIM_OUTPUTS"] = str(outputs_listing(name, fresh=True))
            process = subprocess.Popen(
                [sys.executable, str(Path(__file__)), "--trace", str(trace), str(selected[name])],
                cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            running[name] = (process, time.monotonic(), log, expected, reuse, slot)
            in_use += expected
        time.sleep(0.2)
        for name, (process, started, log, _, reused, slot) in list(running.items()):
            elapsed = time.monotonic() - started
            if process.poll() is None and elapsed < timeout:
                continue
            if process.poll() is None:
                os.killpg(process.pid, 9)
                process.wait()
                print(f"--- {name}: timed out after {timeout}s", file=log)
            log.close()
            slot.close()
            del running[name]
            passed = process.returncode == 0
            if not passed and reused and records.get(name, {}).get("status") != "fail":
                # A new failure might come from a reused oracle result: run it again from scratch.
                retried.add(name)
                queue.insert(0, name)
                print(f"retry {name} without reused oracle results", flush=True)
                continue
            trace_path = traces / f"{name}.json"
            record = {"status": "pass" if passed else "fail", "seconds": round(elapsed, 1),
                      "finished": time.strftime("%Y-%m-%dT%H:%M:%S")}
            # A harness that only passes fresh no longer reuses oracle results.
            record["oracle_reuse"] = records.get(name, {}).get("oracle_reuse", True) and not (passed and name in retried)
            try:
                trace = json.loads(trace_path.read_text())
            except (OSError, ValueError):
                trace = None  # killed before writing it, or the disk was full: the pass can't be keyed
            if trace:
                record["dependencies"], record["artifacts"] = dependencies(trace)
                record["peak_bytes"] = trace["peak_bytes"]
                record["key"] = key(record, hasher, environment_digest)
                if passed:
                    clear_large_outputs(name, trace)
            else:
                record["dependencies"], record["artifacts"] = [], []
            records[name] = record
            results[name] = record
            save_state(state)
            publish(name, record)
            tail = "" if passed else f"  ({logs / (name + '.log')})"
            print(f"{'PASS' if passed else 'FAIL'} {name} {elapsed:.0f}s{tail}", flush=True)
    return results, time.monotonic() - started_all


def stale(name, state, hasher, environment):
    record = state["harnesses"].get(name)
    if record and record.get("status") == "pass" and "key" in record and key(record, hasher, environment) == record["key"]:
        return False
    # Passed elsewhere with the same inputs? Try this worktree's and the shared dependency lists.
    candidates = [record] if record and record.get("dependencies") else []
    template = SHARED / "dependencies" / f"{name}.json"
    if template.exists():
        candidates.append(json.loads(template.read_text()))
    for candidate in candidates:
        shared = SHARED / "passes" / name / f"{key(candidate, hasher, environment)}.json"
        if shared.exists():
            state["harnesses"][name] = dict(json.loads(shared.read_text()), shared=True)
            return False
    return True


def trace_main(output, script, arguments):
    """Run one harness and record what it read and executed."""
    import atexit
    import resource
    import runpy
    read, commands = set(), []

    def audit(event, details):
        if event == "open":
            path, mode = details[0], details[1]
            if isinstance(path, (str, bytes, os.PathLike)) and (mode is None or "r" in str(mode) and "+" not in str(mode)):
                read.add(os.fsdecode(path))
        elif event == "subprocess.Popen":
            arguments_ = details[1]
            arguments_ = [arguments_] if isinstance(arguments_, (str, bytes, os.PathLike)) else list(arguments_ or [])
            commands.append([os.fsdecode(a) if isinstance(a, (bytes, os.PathLike)) else str(a) for a in arguments_])

    def write():
        modules = [getattr(module, "__file__", None) for module in list(sys.modules.values())]
        peak = max(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss, resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)
        Path(output).write_text(json.dumps({
            "modules": sorted({m for m in modules if m and inside(m, ROOT)}),
            "read": sorted(path for path in read if os.path.isabs(path) or not path.startswith("<")),
            "commands": commands,
            "peak_bytes": peak * 1024,
        }))

    atexit.register(write)
    sys.addaudithook(audit)
    sys.argv = [script, *arguments]
    sys.path.insert(0, str(Path(script).parent))
    runpy.run_path(script, run_name="__main__")


def finished_run(failed):
    """Record that this run completed, for tools/gate.py: a crashed runner never writes it."""
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    (STATE_DIR / "last-run.json").write_text(json.dumps({"run": os.environ.get("PARITY_RUN_ID"), "failed": failed}))


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--trace":
        return trace_main(sys.argv[2], sys.argv[3], sys.argv[4:])
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("names", nargs="*", help="harness names, e.g. food or block_update")
    parser.add_argument("--all", action="store_true", help="run every harness from scratch, ignoring earlier passes")
    parser.add_argument("--fresh", action="store_true", help="run the vanilla oracles even if their inputs are unchanged")
    parser.add_argument("--failed", action="store_true", help="rerun the harnesses that failed last time")
    parser.add_argument("--list", action="store_true", help="print what would run, then stop")
    parser.add_argument("--status", action="store_true", help="print each harness's last result")
    parser.add_argument("-j", "--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 3),
                        help="harnesses at once in this run; every runner on the machine shares PARITY_MACHINE_JOBS slots")
    parser.add_argument("--timeout", type=int, default=3600, help="seconds per harness")
    args = parser.parse_args()

    found = harnesses()
    state = load_state()
    hasher = Hasher(state["hashes"])
    environment = environment_key(hasher)
    records = state["harnesses"]
    unknown = [name for name in args.names if name not in found]
    if unknown:
        parser.error("unknown harnesses: " + ", ".join(unknown))
    if args.status:
        for name in found:
            record = records.get(name)
            status = "never run" if not record else record["status"] + (" (stale)" if stale(name, state, hasher, environment) and record["status"] == "pass" else "")
            seconds = f"{record['seconds']:>7.0f}s" if record else ""
            print(f"{name:<40} {status:<15} {seconds}")
        save_state(state)
        return 0
    if args.names:
        chosen = args.names
    elif args.all:
        chosen = list(found)
    elif args.failed:
        chosen = [name for name in found if records.get(name, {}).get("status") == "fail"]
    else:
        chosen = [name for name in found if stale(name, state, hasher, environment)]
    selected = {name: found[name] for name in chosen}
    if args.list or not selected:
        for name in selected:
            print(name)
        print(f"{len(selected)} of {len(found)} harnesses would run" if args.list else "Nothing to run: every harness passed with its current inputs.")
        save_state(state)
        if not args.list:
            finished_run([])
        return 0

    jobs = max(1, args.jobs)
    # Harnesses that never ran here have no recorded probes: build them all.
    artifacts = [["all", "", "release"]] if any("artifacts" not in records.get(name, {}) for name in selected) else []
    for name in selected:
        for artifact in records.get(name, {}).get("artifacts", []):
            if artifact not in artifacts:
                artifacts.append(artifact)
    print("Building probes...", flush=True)
    started = time.monotonic()
    built, errors = prebuild(artifacts, os.cpu_count() or 8)
    if not built:
        print(errors[-4000:])
        print("Probe build failed; harnesses will report their own build errors.", flush=True)
    print(f"Probes built in {time.monotonic() - started:.0f}s. Running {len(selected)} harnesses, {jobs} at a time...", flush=True)
    reuse_oracles = not (args.fresh or args.all)
    results, elapsed = run(selected, state, hasher, environment_key(hasher), jobs, args.timeout, int(memory_available() * 0.75),
                           reuse_oracles)
    save_state(state)
    failed = sorted(name for name, record in results.items() if record["status"] != "pass")
    finished_run(failed)
    print(f"\n{len(results) - len(failed)} passed, {len(failed)} failed in {elapsed / 60:.1f} min.")
    for name in failed:
        print(f"  {name}: {STATE_DIR / 'logs' / (name + '.log')}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
