#!/usr/bin/env python3
"""Profile the asset/block-facts/behaviour loading phase in release executables.

Build `shared_json_startup_profile` with the same Cargo release profile at each
revision. With --a01-binary, --cf-binary and --cd-binary, the script compares
five versions in alternated separate processes; --baseline-binary optionally
supplies a build from the original pre-PR checkout. Without historical inputs,
legacy baseline/pr607/shared/deferred same-binary comparisons are supported.
The measurement excludes full client startup, renderer and first frame.
"""
import argparse
import hashlib
import json
import math
import os
import statistics
import sys
import subprocess
import tempfile
from pathlib import Path

try:
    from . import lab
except ImportError:  # run as a script: its folder holds lab.py
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import lab


def _measurement(fields, field, binary, mode):
    """Parse one child-reported numeric field, rejecting invalid measurements.

    Elapsed times and RSS counters are durations/sizes: NaN, infinities,
    negative values and non-numeric text are never valid samples. Reject them
    with a diagnostic naming the binary, mode and field rather than letting a
    bad value reach aggregation or a JSON report.
    """
    if field not in fields:
        raise RuntimeError(f"{binary} {mode}: missing required measurement {field}")
    raw = fields[field]
    try:
        value = float(raw)
    except (TypeError, ValueError) as exc:
        raise RuntimeError(f"{binary} {mode}: invalid {field}: {raw!r} is not numeric") from exc
    if not math.isfinite(value):
        raise RuntimeError(f"{binary} {mode}: invalid {field}: {raw!r} is not finite")
    if value < 0:
        raise RuntimeError(f"{binary} {mode}: invalid {field}: {raw!r} is negative")
    return value


def _derived(value, field, binary, mode):
    """Guard the wait4-derived measurements before they enter a report."""
    if not math.isfinite(value) or value < 0:
        raise RuntimeError(f"{binary} {mode}: invalid {field}: {value!r} is not finite and nonnegative")
    return value


def run(binary, resources, mode):
    # Drain both streams to seekable files, never to an undrained PIPE: a child
    # that writes more than a pipe buffer to stderr would block forever while
    # wait4 waits for it to exit. A temp file also keeps the stderr available
    # for the failure diagnostic without a reader thread.
    with tempfile.TemporaryFile(mode="w+t") as output, \
            tempfile.TemporaryFile(mode="w+b") as errors:
        proc = subprocess.Popen([str(binary), str(resources), mode], stdout=output,
                                stderr=errors)
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        if proc.returncode:
            errors.seek(0)
            raise RuntimeError(f"{binary} {mode}: " + errors.read().decode(errors="replace"))
        output.seek(0)
        fields = dict(line.rstrip().split("=", 1) for line in output if "=" in line)
    measurements = {
        "phase_ms": _measurement(fields, "phase_ms", binary, mode),
        "total_ms": _measurement(fields, "total_ms", binary, mode),
        "cpu_ms": _derived((usage.ru_utime + usage.ru_stime) * 1000.0, "cpu_ms", binary, mode),
        "peak_rss_kib": _derived(usage.ru_maxrss, "peak_rss_kib", binary, mode),
    }
    for field in ("behaviour_json_ms", "facts_ms", "behaviours_ms", "post_rss_kib"):
        if field in fields:
            measurements[field] = _measurement(fields, field, binary, mode)
    return measurements


def percentile(values, percent):
    ordered = sorted(values)
    position = (len(ordered) - 1) * percent
    low = int(position)
    return ordered[low] + (ordered[min(low + 1, len(ordered) - 1)] - ordered[low]) * (position - low)


def sha256_file(path):
    """Hash a file's current bytes; used to pin executable identity."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/examples/shared_json_startup_profile"))
    parser.add_argument("--resources", type=Path, default=lab.RESOURCES)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--reference-binary", type=Path, help="Legacy comparison to cf837118")
    parser.add_argument("--baseline-binary", type=Path, help="Optional original main build; expects baseline mode")
    parser.add_argument("--a01-binary", type=Path, help="Release executable built at a01d47f2")
    parser.add_argument("--cf-binary", type=Path, help="Release executable built at cf837118")
    parser.add_argument("--cd-binary", type=Path, help="Release executable built at cd61e11b")
    parser.add_argument("--variant", action="append", metavar="LABEL=PATH@MODE",
                        help="Compare arbitrary release variants, e.g. pool=target/...@deferred")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    binary, resources = args.binary.resolve(), args.resources.resolve()
    if not resources.is_dir():
        parser.error("resources directory missing")

    historical = any((args.a01_binary, args.cf_binary, args.cd_binary, args.baseline_binary))
    if args.variant:
        if historical or args.reference_binary:
            parser.error("--variant cannot be mixed with historical comparison flags")
        commands = {}
        for specification in args.variant:
            if "=" not in specification or "@" not in specification:
                parser.error("--variant must be LABEL=PATH@MODE")
            name, executable_mode = specification.split("=", 1)
            executable, binary_mode = executable_mode.rsplit("@", 1)
            if not name or not executable or not binary_mode or name in commands:
                parser.error("variant names must be unique and all fields nonempty")
            commands[name] = (Path(executable).resolve(), binary_mode)
    elif historical:
        if not all((args.a01_binary, args.cf_binary, args.cd_binary)):
            parser.error("historical comparisons require --a01-binary, --cf-binary and --cd-binary")
        commands = {
            "baseline": ((args.baseline_binary or args.a01_binary).resolve(), "baseline"),
            "a01d47f2": (args.a01_binary.resolve(), "shared"),
            "cf837118": (args.cf_binary.resolve(), "deferred"),
            "cd61e11b": (args.cd_binary.resolve(), "deferred"),
            "current": (binary, "deferred"),
        }
    elif args.reference_binary:
        commands = {
            "baseline": (binary, "baseline"),
            "pr607": (binary, "pr607"),
            "cf837118": (args.reference_binary.resolve(), "deferred"),
            "current": (binary, "deferred"),
        }
    else:
        commands = {mode: (binary, mode) for mode in ("baseline", "pr607", "shared", "deferred")}
    for executable, _ in commands.values():
        if not executable.is_file():
            parser.error(f"release executable missing: {executable}")

    # Pin each executable's identity before the first sample. Several modes can
    # point at one resolved path (same-binary comparisons / alias paths), so pin
    # every distinct path once and report that same identity for all its modes.
    frozen_identity = {}
    for executable, _ in commands.values():
        if executable not in frozen_identity:
            frozen_identity[executable] = sha256_file(executable)

    def verify_identity(executable):
        """Reject a persistent change to an executable's bytes mid-profile."""
        current = sha256_file(executable)
        expected = frozen_identity[executable]
        if current != expected:
            raise RuntimeError(
                f"executable identity changed while profiling {executable}: "
                f"frozen sha256={expected}, current sha256={current}")

    modes = list(commands)
    samples = {mode: [] for mode in modes}
    orders = []
    initial_load = os.getloadavg()
    for i in range(args.repeats):
        order = modes if i % 2 == 0 else list(reversed(modes))
        orders.append(list(order))
        for mode in order:
            executable, binary_mode = commands[mode]
            verify_identity(executable)
            samples[mode].append(run(executable, resources, binary_mode))
            verify_identity(executable)
    # Final guard before emitting the report: every sampled path still matches
    # the identity it was frozen with, so the report cannot pair timings with a
    # replacement binary's hash.
    for executable in frozen_identity:
        verify_identity(executable)
    result = {
        "scope": "Assets + BlockFacts + Behaviours loader only; no full startup or first frame",
        "resources": str(resources),
        "repeats": args.repeats,
        "ordering": orders,
        "load_average_before": initial_load,
        "load_average_after": os.getloadavg(),
        "executables": {
            mode: {"path": str(executable), "mode": binary_mode,
                   "sha256": frozen_identity[executable]}
            for mode, (executable, binary_mode) in commands.items()
        },
        "metrics": {
            mode: {
                key: {
                    "median": round(statistics.median(values), 3),
                    "p25": round(percentile(values, .25), 3),
                    "p75": round(percentile(values, .75), 3),
                    "min": round(min(values), 3),
                    "max": round(max(values), 3),
                    "samples": [round(value, 3) for value in values],
                }
                for key in rows[0]
                for values in [[row[key] for row in rows]]
            }
            for mode, rows in samples.items()
        },
    }
    # Defense in depth: every value was validated at admission, but refuse to
    # emit nonstandard NaN/Infinity tokens even if a future metric slips past.
    # This raises before printing or writing, so an existing report is kept.
    data = json.dumps(result, indent=2, allow_nan=False)
    print(data)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(data + "\n")


if __name__ == "__main__":
    main()
