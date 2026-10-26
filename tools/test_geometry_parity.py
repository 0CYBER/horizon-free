#!/usr/bin/env python3
"""Compare Horizon simulation geometry with the checksum-pinned vanilla binaries."""
import argparse
from collections import Counter
import json
import math
import os
from pathlib import Path
import platform
import random
import struct
import shutil
import subprocess
import tempfile

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
    rng = random.Random(0x484f52495a4f4e47454f)
    special = [0.0, -0.0, 1.0, -1.0, 1e-7, -1e-7,
               math.nextafter(1e-7, 0), math.nextafter(1e-7, math.inf),
               9.999999747378752e-6, math.nextafter(9.999999747378752e-6, 0),
               float.fromhex("0x0.0000000000001p-1022"), 1e-200, 1e200,
               float("inf"), float("-inf"), float("nan")]
    def component():
        return rng.choice(special) if rng.randrange(5) == 0 else rng.uniform(-32, 32)
    def vector():
        return [component() for _ in range(3)]
    boundaries = [-1.0, -0.0, 0.0, 1e-8, .5, math.nextafter(1.0, 0.0), 1.0, math.nextafter(1.0, math.inf)]
    for start in boundaries:
        for end in boundaries:
            for axes in range(1, 8):
                a = [start if axes & (1 << i) else 0.0 for i in range(3)]
                b = [end if axes & (1 << i) else 0.0 for i in range(3)]
                yield "traverse " + numbers(a + b) + " 0"
    for step in range(8000):
        base = rng.choice([0, -30_000_000, 30_000_000, -2147483648, 2147483647])
        start = [base + rng.choice([rng.uniform(-4, 4), -1, 0, .5, 1]) for _ in range(3)]
        end = [v + rng.choice([-16, -1, 0, 1, 16, rng.uniform(-16, 16)]) for v in start]
        if step % 13 == 0:
            end = start[:]
        yield "traverse " + numbers(start + end) + f" {rng.choice([0, 1, 2, 3, 10, 1000])}"
    yield "trig-table"
    for step in range(16000):
        angle = component() if step % 3 == 0 else rng.uniform(-100000, 100000)
        pitch, yaw = rng.uniform(-90, 90), rng.uniform(-360, 360)
        if step % 11 == 0:
            pitch, yaw = rng.choice(special), rng.choice(special)
        yield "rotation " + numbers(vector() + [angle, pitch, yaw]) + f" {rng.randrange(8)} {rng.randrange(3)} {rng.randrange(6)} " + number(component())
    for a in special:
        for b in special:
            yield "vec " + numbers([a, a, a, b, b, b, rng.choice(special)])
    for _ in range(7000):
        a = vector()
        b = a if rng.randrange(7) == 0 else vector()
        yield "vec " + numbers(a + b + [component()])
    for _ in range(7000):
        yield "box " + numbers(vector() + vector() + vector() + vector() + vector() + vector() + vector()) + f" {rng.randrange(3)} " + number(component())
    # Direct ray ties, strict endpoint exclusion, inside and epsilon thresholds.
    for start, end in [([-1, -1, -1], [2, 2, 2]), ([2, 2, 2], [-1, -1, -1]),
                       ([-1, .5, .5], [0, .5, .5]), ([0, .5, .5], [1, .5, .5]),
                       ([.5, .5, .5], [2, .5, .5]), ([-1e-7, .5, .5], [1e-7, .5, .5])]:
        yield "box " + numbers([0, 0, 0, 1, 1, 1] * 2 + start + end + [0, 0, 0]) + " 0 " + number(0)
    integers = [-2147483648, -2147483647, -16, -1, 0, 1, 15, 16, 2147483646, 2147483647]
    for a in integers:
        for b in integers:
            yield f"block-box {a} {b} {a} {b} {a} {b}"
    shape_id = 0
    for scenario in range(180):
        cube = scenario % 3 == 0
        sizes = [rng.randrange(1, 7) for _ in range(3)] if scenario else [0, 0, 0]
        if scenario == 0:
            cube = False
        coords = []
        for size in sizes:
            if cube:
                coords.append([i / size for i in range(size + 1)])
            else:
                coords.append(sorted(rng.sample(range(-16, 33), size + 1)))
                coords[-1] = [v / 16 for v in coords[-1]]
        if scenario == 1:
            sizes = [1, 1, 1]
            coords = [[0.0, 1.0] for _ in range(3)]
        density = rng.choice([0, .15, .5, .85, 1]) if scenario != 1 else 1
        occupied = "".join("1" if rng.random() < density else "0" for _ in range(math.prod(sizes))) or "-"
        yield f"grid {int(cube)} {' '.join(map(str, sizes))} " + numbers(sum(coords, [])) + " " + occupied
        for step in range(250):
            # Collision boxes lie outside, touch, or overlap individual boundaries.
            lower, upper = [], []
            for axis in range(3):
                edge = rng.choice(coords[axis])
                offset = rng.choice([0, 1e-7, -1e-7, 1e-8, -1e-8, .3, -.3, -1, 1])
                lower.append(edge + offset)
                upper.append(edge + offset + rng.choice([0, 1e-8, 1e-7, .2, .6, 1.8]))
            point = [rng.choice(coords[axis] + special + [rng.uniform(-3, 3)]) for axis in range(3)]
            block = rng.choice([(0, 0, 0), (-17, -64, -1), (30_000_000, 64, -30_000_000)])
            start = [rng.uniform(-3, 3) + v for v in block]
            end = [rng.uniform(-3, 3) + v for v in block]
            if step % 13 == 0:
                start = [.5 + v for v in block]
            if step % 17 == 0:
                end = start[:]
            if step % 19 == 0:
                end = [v + 1e-5 for v in start]
            amount = rng.choice(special) if step % 4 == 0 else rng.uniform(-4, 4)
            yield f"query {shape_id} " + numbers(lower + upper + start + end + point) + f" {' '.join(map(str, block))} {rng.randrange(3)} " + number(amount)
        shape_id += 1
    for _ in range(2500):
        count = rng.randrange(9)
        selected = [rng.randrange(shape_id) for _ in range(count)]
        yield "many " + numbers(vector() + vector()) + f" {rng.randrange(3)} " + number(component()) + f" {count}" + (" " + " ".join(map(str, selected)) if selected else "")
    for step in range(12000):
        count = rng.randrange(12)
        selected = [rng.randrange(shape_id) for _ in range(count)]
        movement = vector() if step % 3 == 0 else [rng.choice([0.0, -0.0, 1e-8, -1e-8, .3, -.3, 1.0, -1.0]) for _ in range(3)]
        lower = [rng.choice([-1.0, -.6, -.5, -0.0, 0.0, .5, 1.0]) for _ in range(3)] if step % 4 else vector()
        upper = [v + rng.choice([0.0, .6, 1.0, 1.8]) for v in lower]
        max_step = rng.choice([0.0, -0.0, .5, .6, 1.0, 2.0, float("nan"), float("inf")])
        current_y = rng.choice([0.0, -0.0, -.5, .5, -.6, .6, float("nan")])
        yield "motion " + numbers(lower + upper + movement + [max_step, current_y]) + f" {count}" + (" " + " ".join(map(str, selected)) if selected else "")
    epsilon_shapes = []
    for offset in [0.0, -0.0, 1e-8, -1e-8, 1e-7, math.nextafter(1e-7, 0), math.nextafter(1e-7, math.inf), -1e-7, 1.0-1e-7, 1.0, 1.0+1e-7, 2.0]:
        epsilon_shapes.append(shape_id)
        shape_id += 1
        yield "grid 0 1 1 1 " + numbers([offset, offset+1, 0, 1, 0, 1]) + " 1"
    for first in epsilon_shapes:
        for second in epsilon_shapes:
            for operation in range(0, 16, 2):
                yield f"join {first} {second} {operation}"
                shape_id += 1
    # Joined shapes feed subsequent joins and collision queries. Sparse and empty
    # results retain merged coordinate metadata, which affects later operations.
    for step in range(4500):
        first = rng.randrange(shape_id)
        second = first if step % 7 == 0 else rng.randrange(shape_id)
        operation = step % 16
        yield f"join {first} {second} {operation}"
        if operation & 1:
            continue
        new_id = shape_id
        shape_id += 1
        for axis in range(3):
            lower = [rng.uniform(-2, 2) for _ in range(3)]
            upper = [v + rng.choice([.2, .6, 1.8]) for v in lower]
            start, end = [rng.uniform(-2, 2) for _ in range(3)], [rng.uniform(-2, 2) for _ in range(3)]
            point = [rng.choice([0, .5, 1, rng.uniform(-2, 2), float("nan")]) for _ in range(3)]
            yield f"query {new_id} " + numbers(lower + upper + start + end + point) + f" 0 0 0 {axis} " + number(rng.uniform(-3, 3))
    for step in range(600):
        lower = [rng.choice([0, -0.0, -.5, .125, .25, .5, .875, 1, 1e-8, -1e-8, 1e-7, -1e-7]) for _ in range(3)]
        upper = [v + rng.choice([0, 1e-8, 1e-7, math.nextafter(1e-7, math.inf), .125, .25, .5, 1, .3333333333333333]) for v in lower]
        if step % 31 == 0:
            lower, upper = [0.0] * 3, [1.0] * 3
        yield "make-box " + numbers(lower + upper)
        shape_id += 1
    for step in range(1000):
        first = rng.randrange(shape_id)
        if step % 2:
            second = rng.randrange(shape_id)
            operation = (step // 2) % 16
            yield f"join-optimized {first} {second} {operation}"
            if operation & 1:
                continue
        else:
            yield f"optimize {first}"
        new_id = shape_id
        shape_id += 1
        lower, upper = [-.6, 0, .2], [-.2, 1.8, .8]
        yield f"query {new_id} " + numbers(lower + upper + [-1, .5, .5, 2, .5, .5, .5, .5, .5]) + " 0 0 0 0 " + number(1)


    yield from step_corpus(rng, shape_id, vector)
    yield from sweep_corpus(rng, vector)
    for _ in range(12000):
        yield f"floor-height {rng.randrange(shape_id)} {rng.randrange(shape_id)}"


def sweep_corpus(rng, vector):
    for step in range(2400):
        base = rng.choice([-2147483648, -30000000, -17, -1, 0, 16, 30000000, 2147483640])
        a = [base + rng.randrange(4) for _ in range(3)]
        b = [base + rng.randrange(7) for _ in range(3)]
        direction = vector() if step % 5 == 0 else [rng.choice([-2, -1, -0.0, 0.0, 1, 2]) for _ in range(3)]
        limit = rng.choice([0, 1, 3, 10, 1000])
        yield f"block-iterate {' '.join(map(str, a+b))} " + numbers(direction) + f" {limit}"
        lower = [v + rng.choice([0.0, -0.0, .25, .5, .99999999]) for v in a]
        upper = [v + rng.choice([0.0, .25, .6, 1.0, 1.8]) for v in lower]
        yield "box-iterate " + numbers(lower + upper + direction) + f" {limit}"
    float_epsilon = struct.unpack(">f", struct.pack(">f", 1e-5))[0]
    epsilon = math.sqrt(struct.unpack(">f", struct.pack(">f", float_epsilon * float_epsilon))[0])
    for axis in range(3):
        for sign in [-1, 1]:
            for value in [epsilon, math.nextafter(epsilon, 0), math.nextafter(epsilon, math.inf)]:
                end = [0.0] * 3
                end[axis] = sign * value
                yield "sweep " + numbers([0.0] * 3 + end + [-1.0] * 3 + [1.0] * 3) + " 0"
    for step in range(9000):
        base = rng.choice([-30000000, -17, -1, 0, 16, 30000000])
        start = [base + rng.choice([-1.0, -0.0, 0.0, .5, 1.0, rng.uniform(-3, 3)]) for _ in range(3)]
        delta = [rng.choice([-4, -1, -1e-5, -1e-8, -0.0, 0.0, 1e-8, 1e-5, 1, 4, rng.uniform(-5, 5)]) for _ in range(3)]
        if step % 19 == 0:
            delta = [rng.choice([0.0, 1e-200, epsilon, math.nextafter(epsilon, 0), math.nextafter(epsilon, math.inf)]), 0.0, 0.0]
        end = [a+b for a, b in zip(start, delta)]
        lower = [end[i] + rng.choice([-1.0, -.3, -0.0, 0.0]) for i in range(3)]
        upper = [v + rng.choice([0.0, .3, .6, 1.0, 1.8, 2.0]) for v in lower]
        yield "sweep " + numbers(start + end + lower + upper) + f" {rng.choice([0, 1, 2, 4, 10, 1000])}"
    # Packed-position identity aliases Y every 4096 blocks. The initial loop
    # still visits all cells; subsequent footprints use the packed dedup set.
    for delta in [0.0, 1e-8, 1.0, -1.0]:
        yield "sweep " + numbers([0, 0, 0, delta, 0, 0, delta, -2048, 0, delta+.3, 2048, .3]) + " 0"


def step_corpus(rng, shape_id, vector):
    def selected(ids):
        return str(len(ids)) + (" " + " ".join(map(str, ids)) if ids else "")
    # Controlled shape selection invokes the real Entity.collide, including its
    # second query. Ledges, floors and ceilings exercise success and failure.
    ledges = []
    for height in [.125, .5, .6, 1.0, 1.5, math.nextafter(.5, 0), math.nextafter(.5, math.inf)]:
        ledges.append(shape_id)
        yield "grid 0 1 1 1 " + numbers([0, 1, 0, height, 0, 1]) + " 1"
        shape_id += 1
    floor = shape_id
    yield "grid 0 1 1 1 " + numbers([-8, 8, -1, 0, -8, 8]) + " 1"
    shape_id += 1
    ceiling = shape_id
    yield "grid 0 1 1 1 " + numbers([-8, 8, 2.1, 3, -8, 8]) + " 1"
    shape_id += 1
    for ledge in ledges:
        for height in [0.0, .5, .6, 1.0, 2.0]:
            for y in [0.0, .2, 1.0]:
                for dy in [0.0, -.1, -2.0, .1]:
                    for grounded in [0, 1]:
                        for overhang in [False, True]:
                            blocks = [floor, ledge] + ([ceiling] if overhang else [])
                            bounds = [-.6, y, .2, -.2, y+1.8, .8]
                            yield "step-motion " + numbers(bounds + [1.0, dy, 0.0, height]) + f" {grounded} 0 " + selected(blocks) + " " + selected(blocks) + " -1"
    for step in range(16000):
        def random_shapes():
            return [rng.randrange(shape_id) for _ in range(rng.randrange(9))]
        entities, initial, stepping = random_shapes(), random_shapes(), random_shapes()
        if step % 3 == 0:
            initial = stepping = [floor, rng.choice(ledges)]
        lower = [rng.choice([-1.0, -.6, -.5, -0.0, 0.0, .5, 1.0]) for _ in range(3)]
        if step % 11 == 0:
            lower = vector()
        upper = [v + rng.choice([0.0, .6, 1.0, 1.8]) for v in lower]
        movement = vector() if step % 4 == 0 else [rng.choice([0.0, -0.0, 1e-200, 1e-8, -1e-8, .3, -.3, 1.0, -1.0]) for _ in range(3)]
        height = rng.choice([0.0, -0.0, -.6, .5, .6, 1.0, 2.0, float("nan"), float("inf")])
        border = rng.randrange(shape_id) if step % 5 == 0 else -1
        yield "step-motion " + numbers(lower + upper + movement + [height]) + f" {rng.randrange(2)} " + " ".join(map(selected, [entities, initial, stepping])) + f" {border}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    directory = lab.LOCAL / "rust-geometry-parity"
    directory.mkdir(parents=True, exist_ok=True)
    report_path = directory / "report.json"
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = test_rust_parity.verify_artifacts()
    for earlier in directory.glob("run-*"):  # one run per run used to stay behind: 5 GB here after a few weeks
        shutil.rmtree(earlier, ignore_errors=True)
    base = Path(tempfile.mkdtemp(prefix="run-", dir=directory))
    commands = list(corpus())
    inputs = base / "inputs.txt"
    inputs.write_text("\n".join(commands) + "\n", encoding="utf-8")
    oracle = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaGeometryOracle.java"
    sources = [oracle, Path(__file__).resolve(), lab.ROOT / "tools/test_rust_parity.py", lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock", lab.ROOT / "crates/horizon-core/Cargo.toml", *sorted((lab.ROOT / "crates/horizon-core/src").rglob("*.rs"))]
    tested_sources = test_rust_parity.source_hashes(sources)
    classes = base / "classes"
    classes.mkdir()
    dependency_path = os.pathsep.join(map(str, libraries))
    test_rust_parity.run(["javac", "--release", "25", "-proc:none", "-cp", os.pathsep.join([str(jars["server"]), dependency_path]), "-d", str(classes), str(oracle)])
    test_rust_parity.run(["cargo", "build", "--release", "--bin", "horizon-geometry-parity"])
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=lab.ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "release" / ("horizon-geometry-parity.exe" if os.name == "nt" else "horizon-geometry-parity")
    candidate = base / "rust.txt"
    with inputs.open("rb") as stdin, candidate.open("wb") as stdout:
        test_rust_parity.run([str(binary)], stdin=stdin, stdout=stdout)
    references = []
    for side, jar in jars.items():
        reference = base / f"vanilla-{side}.txt"
        with (base / f"vanilla-{side}.log").open("wb") as log:
            test_rust_parity.run(["java", "-Xmx512M", "-cp", os.pathsep.join([str(classes), str(jar), dependency_path]), "dev.horizonlab.parity.VanillaGeometryOracle", str(inputs), str(reference)], stdout=log, stderr=subprocess.STDOUT)
        matched = test_rust_parity.compare(reference, candidate)
        references.append({"side": side, "jar_sha256": lab.digest(jar, "sha256"), "output_sha256": lab.digest(reference, "sha256"), "matched_output_lines": matched})
        print(f"{side}: {matched:,} geometry output lines match", flush=True)
    test_rust_parity.ensure_sources_unchanged(sources, tested_sources)
    report = {"schema": 1, "minecraft": lock["minecraft"]["version"], "manifest_sha1": lock["minecraft"]["manifest_sha1"], "platform": platform.platform(),
              "java": subprocess.check_output(["java", "--version"], text=True).splitlines()[0], "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
              "commands": dict(Counter(command.split()[0] for command in commands)), "inputs_sha256": lab.digest(inputs, "sha256"), "rust_output_sha256": lab.digest(candidate, "sha256"),
              "sources_sha256": tested_sources, "references": references,
              "scope": "Selected Vec3/AABB operations, sine table entries, quantized trig, rotations and look directions; finite block-ray traversal; bounded inclusive/directional block iteration and finite swept-block visits with step indices; immutable cube/array voxel-grid bounds, column minima/maxima, greedy boxes, collide, ray hits, closest points and cell lookup; lazy floor-height selection; box quantization, optimized/unoptimized Boolean joins, alias identity and joinIsNotEmpty; Entity.collideWithShapes, candidate step heights and Entity.collide step-up decisions/query bounds with caller-controlled block/entity/border shape lists. No live entities, actual world shape selection, grounding/velocity side effects, fluids or block behavior."}
    lab.write_json(report_path, report)
    lab.write_json(base / "report.json", report)
    print(f"Geometry comparison passed: {report_path}")


if __name__ == "__main__":
    main()
