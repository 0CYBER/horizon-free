#!/usr/bin/env python3
"""Compare 26.3 base noise sampling against both checksum-pinned vanilla JARs."""
import argparse
from collections import Counter
import itertools
import json
import math
import os
from pathlib import Path
import platform
import random
import struct
import subprocess

try:
    from . import lab, test_rust_parity
except ImportError:
    import lab
    import test_rust_parity


def number(value):
    return struct.pack(">d", value).hex()


def numbers(values):
    return " ".join(map(number, values))


def corpus():
    rng = random.Random(0x484f52495a4f4e4e4f495345)
    half = math.nextafter(2**24, 0)
    specials = [0.0, -0.0, 1.0, -1.0, .5, -.5, 1e-7, -1e-7,
                float.fromhex("0x0.0000000000001p-1022"), 1e-200, -1e-200,
                2**24, -(2**24), half, -half, math.nextafter(half, 0),
                math.nextafter(-half, -math.inf), 2**25, -(2**25),
                2147483647.0, -2147483648.0, 1e100, -1e100,
                float("inf"), float("-inf"), float("nan")]
    for value in specials:
        yield "wrap " + number(value)
    for _ in range(12000):
        yield "wrap " + (f"{rng.getrandbits(64):016x}" if rng.randrange(2) else number(rng.uniform(-1e10, 1e10)))
    seeds = [-(2**63), -1, 0, 1, 2**63 - 1, *[rng.randrange(-2**63, 2**63) for _ in range(19)]]
    scales = [0.0, -0.0, .001, .125, .5, 1.0, 4.0, -.125, -4.0, 1e-200, 1e200, math.inf, -math.inf, math.nan]
    derivative_edges = ["00000000", "80000000", "3f800000", "bf800000", "7f800000", "ff800000", "7fc00000", "00000001", "7f7fffff"]
    sampler_id = 0
    valid_bases = []
    invalid_ranges = []
    for stream, (kind, skip), scenario in itertools.product(["legacy", "xoroshiro"], [("perlin", 0), ("simplex", 0), ("simplex", 1), ("smeared", 0)], range(48)):
        seed = seeds[scenario % len(seeds)]
        scale = scales[scenario % len(scales)]
        (invalid_ranges if kind == "smeared" and math.isnan(scale) else valid_bases).append(sampler_id)
        yield f"create {kind} {stream} {seed} {skip} {number(scale)}"
        for value in specials:
            # Axis-specific extremes as well as all coordinates equal.
            for axis in range(4):
                coords = [value if i == axis or axis == 3 else 0.0 for i in range(3)]
                yield f"query {sampler_id} " + numbers(coords) + " 00000000 80000000 3f800000"
        for step in range(400):
            if step % 5 == 0:
                coords = [rng.choice(specials) for _ in range(3)]
            elif step % 7 == 0:
                value = rng.uniform(-2, 2)
                coords = [value, value, value]  # Ties in skipped-offset simplex.
            else:
                magnitude = rng.choice([1, 16, 10000, 30_000_000, 2**31, 1e100])
                coords = [rng.uniform(-magnitude, magnitude) for _ in range(3)]
            initial = [rng.choice(derivative_edges) if step % 3 == 0 else f"{rng.getrandbits(32):08x}" for _ in range(3)]
            yield f"query {sampler_id} " + numbers(coords) + " " + " ".join(initial)
        local_edges = [-1.0, math.nextafter(-1.0, -math.inf), -0.0, 0.0, math.nextafter(0.0, math.inf),
                       .125, .5, math.nextafter(1.0, 0.0), 1.0, math.nextafter(1.0, math.inf)]
        for x, y in itertools.product(local_edges, repeat=2):
            yield f"local {sampler_id} " + numbers([x, y, x]) + " 00000000 80000000 3f800000"
        if kind in ["perlin", "smeared", "simplex"]:
            for step in range(32):
                sizes = [rng.randrange(1, 6), rng.randrange(1, 17), rng.randrange(1, 6)]
                steps = [rng.choice([1, 4, 16, 2147483647]) for _ in range(3)]
                minimum = [rng.choice([-2147483648, 2147483647, -64, 0, 16]) for _ in range(3)]
                xz_scale, y_scale = (rng.choice([0.0, -0.0, .001, -.001, .5, 1.0, 4.0, 1e100, math.inf, -math.inf, math.nan]) for _ in range(2))
                amplitude = rng.choice(derivative_edges)
                seed = rng.randrange(-2**63, 2**63)
                if step % 11 == 0:
                    sizes[step % 3] = 0 if step == 0 else -1
                if step % 13 == 0:
                    steps[step % 3] = 0 if step == 0 else -1
                yield f"volume {sampler_id} " + " ".join(map(str, sizes + minimum + steps)) + f" {numbers([xz_scale,y_scale])} {amplitude} {seed} {step % 4}"
        sampler_id += 1
    stacks = []
    amplitudes = ["00000000", "80000000", "3f800000", "bf800000", "3e800000", "1e3ce508", "7f7fffff", "7f800000", "ff800000"]
    for scenario in range(640):
        layers = []
        layer_count = 0
        depth = 1
        for _ in range(scenario % 7):
            eligible = [entry for entry in stacks if entry[1] <= 24 and entry[2] <= 3]
            if eligible and rng.randrange(3) == 0:
                selected, count, previous_depth = rng.choice(eligible)
                flatten = rng.randrange(2)
                depth = max(depth, previous_depth + (1 - flatten))
                layer_count += count if flatten else 1
            else:
                selected, flatten = rng.choice(valid_bases), 0
                layer_count += 1
            frequency = rng.choice(specials) if scenario % 4 == 0 else rng.uniform(-8, 8)
            # Keep generated success IDs stable: flattening 0 * infinity would
            # itself create a NaN layer amplitude and reject the build.
            amplitude = rng.choice(amplitudes[2:7] if flatten else amplitudes)
            layers.extend([str(flatten), str(selected), number(frequency), amplitude])
        yield f"stack {scenario % 7} " + " ".join(layers)
        stacks.append((sampler_id, layer_count, depth))
        for step in range(64):
            point = [rng.choice(specials) if step % 5 == 0 else rng.uniform(-1000, 1000) for _ in range(3)]
            yield f"query {sampler_id} " + numbers(point) + " 00000000 00000000 00000000"
        for step in range(3):
            scale = [rng.choice([0.0, .001, .5, -2.0, 1e100, math.inf, math.nan]) for _ in range(2)]
            yield f"volume {sampler_id} 2 4 2 -17 -64 2147483647 4 1 16 " + numbers(scale) + f" {rng.choice(amplitudes)} {scenario} {step + 1}"
        sampler_id += 1
        # Failed builds do not append a sampler. Zero amplitude does not bypass
        # checking a layer's invalid range; NaN frequency is allowed instead.
        if scenario % 16 == 0:
            yield f"stack 1 0 {rng.choice(valid_bases)} {number(1.0)} 7fc00000"
            yield f"stack 1 0 {rng.choice(invalid_ranges)} {number(1.0)} 00000000"
    yield f"stack 1 0 0 {number(1.0)} 00000000"
    yield f"stack 1 1 {sampler_id} {number(1.0)} 7f800000"  # Flattened zero times infinity rejects.
    yield f"stack 1 0 {sampler_id} {number(1.0)} 7f800000"  # Nested range remains zero; samples are NaN.
    yield f"query {sampler_id + 1} " + numbers([1.0, 2.0, 3.0]) + " 00000000 00000000 00000000"
    bases = [-(2**31), 2**31-1, -1075, -1074, -1023, -1022, -32, -16, -8, 0, 16, 32, 1022, 1023, 1024]
    base_amplitudes = [0.0, -0.0, 1.0, 2.0, .5, 1e-6, 1e-160, 1e160, 1e200, 1e-200, -1.0, math.inf, -math.inf, math.nan]
    for scenario in range(1800):
        parity = int(scenario % 5 == 0)
        count = rng.choice([0, 1, 2, 3, 4, 8, 16, 32, 33, 64, -1])
        base = rng.choice(bases) if scenario % 3 == 0 else rng.randrange(-32, 33)
        amplitude = rng.choice(base_amplitudes) if scenario % 3 == 0 else rng.uniform(.0001, 100)
        normalization = scenario % 3
        if scenario >= 1400:
            parity |= 2
            base = rng.choice([-64, -16, -8, -1, 0, 1, 2, 4])
        modifiers = []
        if parity & 1 or scenario % 4 != 0:
            for _ in range(max(0, count)):
                choices = [0.0, -0.0, 1.0, .5, -1.0, rng.uniform(0, 4), 1e-16, 1e16]
                if scenario % 17 == 0:
                    choices.extend([1e200, 1e-200, math.inf, math.nan])
                modifiers.append(rng.choice(choices))
        if scenario % 31 == 0:
            modifiers = [0.0] * max(0, count)
        if scenario % 37 == 0 and not (parity & 1):
            modifiers.append(1.0)  # Also check the builder's modifier-index failure.
        points = [[0, 0, 0], [-17.25, 64.125, 32.5], *[[rng.uniform(-1000, 1000) for _ in range(3)] for _ in range(16)]]
        if scenario % 7 == 0:
            points.extend([[value, value, value] for value in specials])
        for stream in ["legacy", "xoroshiro"]:
            yield f"normal {stream} {seeds[scenario % len(seeds)]} {parity} {base} {count} {normalization} {number(amplitude)} {len(modifiers)} " + numbers(modifiers) + f" {len(points)} " + numbers(sum(points, []))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    build = lab.LOCAL / "rust-noise-parity"
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / "report.json"
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = test_rust_parity.verify_artifacts()
    commands = list(corpus())
    inputs = build / "inputs.txt"
    inputs.write_text("\n".join(commands) + "\n", encoding="utf-8")
    source = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaNoiseOracle.java"
    sources = [source, Path(__file__), lab.ROOT / "tools/test_rust_parity.py", lab.ROOT / "tools/lab.py",
               *sorted((lab.ROOT / "crates/horizon-core").rglob("*.rs")),
               lab.ROOT / "crates/horizon-core/Cargo.toml", lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock"]
    tested_sources = test_rust_parity.source_hashes(sources)
    classes = build / "classes"
    classes.mkdir(exist_ok=True)
    dependency_path = os.pathsep.join(map(str, libraries))
    run = test_rust_parity.run
    run(["javac", "--release", str(lock["minecraft"]["java_major"]), "-proc:none",
         "-cp", os.pathsep.join([str(jars["server"]), dependency_path]), "-d", str(classes), str(source)])
    run(["cargo", "build", "--release", "--bin", "horizon-noise-parity"])
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=lab.ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "release" / ("horizon-noise-parity.exe" if os.name == "nt" else "horizon-noise-parity")
    candidate = build / "rust.txt"
    with inputs.open("rb") as stdin, candidate.open("wb") as stdout:
        run([str(binary)], stdin=stdin, stdout=stdout)
    references = []
    for side, jar in jars.items():
        reference = build / f"vanilla-{side}.txt"
        run(["java", "-Xmx512M", "-cp", os.pathsep.join([str(classes), str(jar), dependency_path]),
             "dev.horizonlab.parity.VanillaNoiseOracle", str(inputs), str(reference)])
        matched = test_rust_parity.compare(reference, candidate)
        references.append({"side": side, "jar_sha256": lab.digest(jar, "sha256"),
                           "output_sha256": lab.digest(reference, "sha256"), "matched_output_lines": matched})
        print(f"{side}: {matched:,} noise output lines match Rust", flush=True)
    test_rust_parity.ensure_sources_unchanged(sources, tested_sources)
    report = {"schema": 1, "minecraft": lock["minecraft"]["version"], "platform": platform.platform(),
              "java": subprocess.check_output(["java", "--version"], text=True).splitlines()[0],
              "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
              "manifest_sha1": lock["minecraft"]["manifest_sha1"], "commands": dict(Counter(line.split()[0] for line in commands)),
              "inputs_sha256": lab.digest(inputs, "sha256"), "rust_output_sha256": lab.digest(candidate, "sha256"),
              "sources_sha256": tested_sources, "references": references,
              "scope": "26.3 gradient offsets, permutation tables, constructor RNG consumption, ranges, coordinate wrap, Perlin 2D/3D and derivative accumulation, simplex 2D/3D with both offset modes, smeared Perlin 2D/3D and inherited unsmeared derivatives; bounded strided density volumes, wrapped block coordinates, flat indices, cached and scalar bulk accumulation with preserved buffer tails; nested and flattened noise stacks; normal-noise octave construction, enabled/disabled/legacy normalization, parity factory, modern positional seeding, legacy Nether biome initialization, failure draw state, scalar and bulk sampling. Finite outputs and signed zeros compared by bits; NaNs canonicalized. No serialized noise definitions, density-function graphs, generated terrain, gameplay, rendering or performance claim."}
    lab.write_json(report_path, report)
    print(f"Scoped noise parity passed: {report_path}")


if __name__ == "__main__":
    main()
