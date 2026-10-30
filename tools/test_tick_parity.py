#!/usr/bin/env python3
"""Compare Rust scheduled-tick traces with checksum-pinned vanilla bytecode."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import tempfile

try:
    from . import lab, test_rust_parity
except ImportError:
    import lab
    import test_rust_parity


def corpus():
    # Priority versus overdue time, first-request deduplication, and callback deferral.
    yield "reset"
    yield "add 0 0"
    yield "add 1 0"
    for command in ["schedule 0 0 64 0 1 3 10", "schedule 1 1 64 0 2 -3 11",
                    "schedule 2 16 64 0 3 0 12", "schedule 0 0 64 0 -100 -3 -99",
                    "save 0 0 0", "tick 10 1 0 1 1 64 0 100", "chunk 0 0",
                    "tick 10 20 1 0 0 64 0 100", "query 0 0 64 0",
                    "tick 10 20 0 0 0 64 0 100"]:
        yield command
    # Saved-list duplicates, negative load order, integer narrowing and time overflow.
    for base in [-9223372036854775808, -2147483648, 0, 2147483647, 9223372036854775807]:
        yield "reset"
        yield "load -1 -1 4 0 -1 64 -1 -2147483648 -100 0 -1 64 -1 2147483647 100 1 -2 65 -2 -1 0 2 -3 66 -3 0 0"
        yield "query 0 -1 64 -1"
        yield "schedule 0 -1 64 -1 0 0 1"
        yield f"save -1 -1 {base}"
        yield f"unpack -1 -1 {base}"
        yield "chunk -1 -1"
        yield f"save -1 -1 {base}"
        yield "tick 9223372036854775807 100 0 0 -1 64 -1 0"
        yield "chunk -1 -1"
    # Equal-key heads, map growth/shrink, chunk removal and heap tie behavior.
    chunks = [(x, z) for x in range(-8, 9) for z in range(-4, 5)]
    for reverse in [False, True]:
        yield "reset"
        for cx, cz in reversed(chunks) if reverse else chunks:
            yield f"add {cx} {cz}"
            for i in range(3):
                yield f"schedule {i} {cx * 16 + i} 64 {cz * 16} {i % 2} 0 0"
        for step in range(18):
            yield f"tick 10 23 0 0 0 64 0 {step}"
            if step % 3 == 0:
                for cx, cz in chunks[step:step + 5]:
                    yield f"remove {cx} {cz}"
            if step % 4 == 0:
                for cx, cz in chunks[step:step + 8]:
                    yield f"restore {cx} {cz}"
        yield "tick 100 10000 0 0 0 64 0 0"
    # Force wraparound probe clusters in the deadline index.
    def mix(key):
        value = key * 0x9e3779b97f4a7c15 & ((1 << 64) - 1)
        value ^= value >> 32
        return value ^ (value >> 16)
    collisions = [x for x in range(1, 2000) if mix(x) & 31 == 31][:18]
    yield "reset"
    for x in collisions:
        yield f"add {x} 0"
        yield f"schedule 0 {x * 16} 64 0 0 0 0"
    for _ in collisions:
        yield "tick 0 1 0 0 0 64 0 0"
    # Active membership, clear/copy, unload and eligibility changes in callbacks.
    for action in range(6):
        yield "reset"
        for cx in range(-2, 4):
            yield f"add {cx} 0"
            for i in range(8):
                yield f"schedule {i % 3} {cx * 16 + i} 64 0 0 {i % 7 - 3} {i}"
        yield f"tick 0 35 {action} 1 1 64 0 9223372036854775806"
        yield "query 1 1 64 0"
        yield "tick 0 100 0 1 1 64 0 0"
        yield "copy -32 0 0 15 100 15 16 0 0"
        yield "clear -30 0 0 -1 100 0"
        for cx in range(-2, 4):
            yield f"chunk {cx} 0"
            yield f"save {cx} 0 -1"
    rng = random.Random(0x484F52495A4F4E5449434B)
    positions = [(x, z) for x in range(-3, 4) for z in range(-3, 4)]
    for scenario in range(8):
        yield "reset"
        for cx, cz in positions:
            yield f"add {cx} {cz}"
        for step in range(2500):
            cx, cz = rng.choice(positions)
            x, y, z = cx * 16 + rng.randrange(8), rng.randrange(-2, 3), cz * 16 + rng.randrange(8)
            kind = rng.randrange(8)
            order = rng.choice([step, -step, 0, -9223372036854775808, 9223372036854775807])
            operation = rng.randrange(100)
            if operation < 55:
                yield f"schedule {kind} {x} {y} {z} {rng.randrange(-5, 50)} {rng.randrange(-7, 8)} {order}"
                if step % 7 == 0:
                    yield f"schedule {kind} {x} {y} {z} {rng.randrange(-10, 100)} 0 {order}"
            elif operation < 65:
                yield f"tick {rng.randrange(-5, 55)} {rng.choice([-1, 0, 1, 2, 7, 17, 100])} {rng.randrange(6)} {kind} {x} {y} {z} {order}"
            elif operation < 70:
                yield f"query {kind} {x} {y} {z}"
            elif operation < 75:
                yield f"enable {cx} {cz} {rng.randrange(2)}"
            elif operation < 80:
                yield f"remove {cx} {cz}"
                yield f"restore {cx} {cz}"
            elif operation < 84:
                yield f"add {cx} {cz}"
            elif operation < 88:
                yield f"reload {cx} {cz} {rng.randrange(-10, 50)} {rng.randrange(-10, 50)}"
            elif operation < 92:
                yield f"save {cx} {cz} {rng.choice([step, -step, -9223372036854775808, 9223372036854775807])}"
            elif operation < 95:
                yield f"chunk {cx} {cz}"
            elif operation < 98:
                yield f"clear {x} -2 {z} {x + rng.randrange(20)} 2 {z + rng.randrange(20)}"
            else:
                yield f"copy {x} -2 {z} {x + 7} 2 {z + 7} 16 0 0"
        for cx, cz in positions:
            yield f"enable {cx} {cz} 1"
        yield "tick 9223372036854775807 100000 0 0 0 0 0 0"
        for cx, cz in positions:
            yield f"chunk {cx} {cz}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    directory = lab.LOCAL / "rust-tick-parity"
    directory.mkdir(parents=True, exist_ok=True)
    report_path = directory / "report.json"
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = test_rust_parity.verify_artifacts()
    base = Path(tempfile.mkdtemp(prefix="run-", dir=directory))
    commands = list(corpus())
    inputs = base / "inputs.txt"
    inputs.write_text("\n".join(commands) + "\n", encoding="utf-8")
    oracle = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaTickOracle.java"
    data_oracle = oracle.with_name("VanillaTickDataOracle.java")
    sources = [oracle, data_oracle, Path(__file__).resolve(), lab.ROOT / "tools/test_rust_parity.py", lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock", lab.ROOT / "crates/horizon-core/Cargo.toml", lab.ROOT / "crates/horizon-world/Cargo.toml", *sorted((lab.ROOT / "crates/horizon-core/src").rglob("*.rs")), *sorted((lab.ROOT / "crates/horizon-world/src").rglob("*.rs")), lab.ROOT / "crates/horizon-world/examples/probe/tick_fixture.rs"]
    tested_sources = test_rust_parity.source_hashes(sources)
    classes = base / "classes"
    classes.mkdir()
    dependency_path = os.pathsep.join(map(str, libraries))
    test_rust_parity.run(["javac", "--release", "25", "-proc:none", "-cp", os.pathsep.join([str(jars["server"]), dependency_path]), "-d", str(classes), str(oracle), str(data_oracle)])
    test_rust_parity.run(["cargo", "build", "--release", "--bin", "horizon-tick-parity"])
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=lab.ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "release" / ("horizon-tick-parity.exe" if os.name == "nt" else "horizon-tick-parity")
    candidate = base / "rust.txt"
    with inputs.open("rb") as stdin, candidate.open("wb") as stdout:
        test_rust_parity.run([str(binary)], stdin=stdin, stdout=stdout)
    def data_java(side, mode):
        completed = subprocess.run(["java", "-Xmx512M", "-cp", os.pathsep.join([str(classes), str(jars[side]), dependency_path]), "dev.horizonlab.parity.VanillaTickDataOracle", mode, str(base)], cwd=lab.ROOT, capture_output=True, text=True, timeout=300)
        (base / f"data-{side}-{mode}.log").write_text(completed.stdout + completed.stderr, encoding="utf-8")
        completed.check_returncode()
        return completed.stdout
    data_java("server", "generate")
    test_rust_parity.run(["cargo", "build", "--release", "-p", "horizon-world", "--example", "world_probe"])
    fixture = Path(metadata["target_directory"]) / "release/examples" / ("world_probe.exe" if os.name=="nt" else "world_probe")
    test_rust_parity.run([str(fixture), "tick_fixture", str(base)])
    references = []
    for side, jar in jars.items():
        reference = base / f"vanilla-{side}.txt"
        with (base / f"vanilla-{side}.log").open("wb") as log:
            test_rust_parity.run(["java", "-Xmx512M", "-cp", os.pathsep.join([str(classes), str(jar), dependency_path]), "dev.horizonlab.parity.VanillaTickOracle", str(inputs), str(reference)], stdout=log, stderr=subprocess.STDOUT)
        matched = test_rust_parity.compare(reference, candidate)
        output = data_java(side, "verify")
        marker = "HORIZON_TICK_DATA_RESULT "
        counts = [json.loads(line.split(marker, 1)[1]) for line in output.splitlines() if marker in line]
        if len(counts) != 1 or counts[0] != {"block_ticks": 251, "fluid_ticks": 251, "region_chunks": 1}:
            raise AssertionError(f"Incomplete tick save fixture: {counts}")
        references.append({"side": side, "jar_sha256": lab.digest(jar, "sha256"), "output_sha256": lab.digest(reference, "sha256"), "matched_output_lines": matched, **counts[0]})
        print(f"{side}: {matched:,} scheduled-tick output lines match", flush=True)
    test_rust_parity.ensure_sources_unchanged(sources, tested_sources)
    report = {"schema": 1, "minecraft": lock["minecraft"]["version"], "manifest_sha1": lock["minecraft"]["manifest_sha1"], "platform": platform.platform(),
              "java": subprocess.check_output(["java", "--version"], text=True).splitlines()[0], "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
              "commands": dict(Counter(command.split()[0] for command in commands)), "inputs_sha256": lab.digest(inputs, "sha256"), "rust_output_sha256": lab.digest(candidate, "sha256"),
              "sources_sha256": tested_sources, "references": references,
              "scope": "Scheduled tick ordering, deduplication, save packing, chunk eligibility/loading and callback mutations only. Block/fluid behavior is not implemented."}
    lab.write_json(report_path, report)
    lab.write_json(base / "report.json", report)
    print(f"Scheduled-tick comparison passed: {report_path}")


if __name__ == "__main__":
    main()
