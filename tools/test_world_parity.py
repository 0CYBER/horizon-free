#!/usr/bin/env python3
"""Exercise Rust NBT, palette, registry and region I/O with pinned vanilla bytecode."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile

try:
    from . import lab, test_rust_parity
except ImportError:
    import lab
    import test_rust_parity


def run(command, log=None):
    result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, capture_output=True, text=True, timeout=300)
    if log:
        log.write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(f"Command failed ({result.returncode}): {command[0]}\n{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    directory = lab.LOCAL / "rust-world-parity"
    directory.mkdir(parents=True, exist_ok=True)
    report_path = directory / "report.json"
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = test_rust_parity.verify_artifacts()
    for earlier in directory.glob("run-*"):  # one run per run used to stay behind: 5 GB here after a few weeks
        shutil.rmtree(earlier, ignore_errors=True)
    base = Path(tempfile.mkdtemp(prefix="run-", dir=directory))
    classes = base / "classes"
    classes.mkdir()
    oracle = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaWorldOracle.java"
    sources = [oracle, Path(__file__).resolve(), lab.ROOT / "tools/test_rust_parity.py", lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock", lab.ROOT / "crates/horizon-world/Cargo.toml",
               *sorted((lab.ROOT / "crates/horizon-world").rglob("*.rs")), *sorted((lab.ROOT / "crates/horizon-core/src").rglob("*.rs"))]
    tested_sources = test_rust_parity.source_hashes(sources)
    dependency_path = os.pathsep.join(map(str, libraries))
    run(["javac", "--release", "25", "-proc:none", "-cp", os.pathsep.join([str(jars["server"]), dependency_path]), "-d", classes, oracle], base / "compile-java.log")

    def java(side, mode):
        return run(["java", "-Xmx1G", "-cp", os.pathsep.join([str(classes), str(jars[side]), dependency_path]),
                    "dev.horizonlab.parity.VanillaWorldOracle", mode, base], base / f"{side}-{mode}.log")

    print("Generating vanilla world-data fixtures...", flush=True)
    print(java("server", "generate").split("HORIZON_WORLD_GENERATED")[-1].strip(), flush=True)
    run(["cargo", "build", "--release", "-p", "horizon-world", "--bin", "horizon-world", "--example", "world_probe"], base / "compile-rust.log")
    metadata = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1"]))
    target = Path(metadata["target_directory"]) / "release"
    suffix = ".exe" if os.name == "nt" else ""
    binary = target / ("horizon-world" + suffix)
    run([target / "examples" / ("world_probe" + suffix), "world_fixture", base], base / "rust-fixture.log")
    for network in [False, True]:
        kind = "network" if network else "nbt"
        output = base / "rust" / kind
        output.mkdir(parents=True)
        for source in range(1, 5):
            for destination in range(1, 5):
                run([binary, "nbt-network-copy" if network else "nbt-copy", base / kind / f"{source}.bin",
                     output / f"{source}-{destination}.bin", source, destination])
    for source in range(1, 5):
        original = base / "regions" / str(source) / "r.-1.0.mca"
        for variant in ["copy", "1", "2", "3", "4"]:
            output = base / "rust/regions" / f"{source}-{variant}" / original.name
            command = [binary, "copy-region", original, output]
            if variant != "copy":
                command.append(variant)
            run(command)
        output = base / "rust/regions" / f"{source}-edited" / original.name
        run([binary, "edit-block-data", original, output, -512, -64, 0, "minecraft:stone"])
        actual = run([binary, "get-block", output, -512, -64, 0]).strip()
        if actual != "minecraft:stone":
            raise AssertionError(f"CLI returned unexpected edited block: {actual}")
        # A second export must fail without changing the existing output.
        before = lab.digest(output, "sha256")
        attempt = subprocess.run([str(binary), "copy-region", str(original), str(output)], capture_output=True, timeout=30)
        if attempt.returncode == 0 or lab.digest(output, "sha256") != before:
            raise AssertionError("Existing region output was not protected")
    references = []
    for side in ["server", "client"]:
        print(f"Checking Rust-written world data in vanilla {side}...", flush=True)
        output = java(side, "verify")
        marker = "HORIZON_WORLD_RESULT "
        results = [json.loads(line.split(marker, 1)[1]) for line in output.splitlines() if marker in line]
        expected = {"nbt_pairs": 32, "palettes": 44, "palette_values": 123776,
                    "region_chunks": 48, "registry_states": 35723, "resident_values": 8192}
        if len(results) != 1 or any(results[0].get(key) != value for key, value in expected.items()):
            raise AssertionError(f"Incomplete world fixture: {results}")
        references.append({"side": side, "jar_sha256": lab.digest(jars[side], "sha256"), **results[0]})
        print(json.dumps(references[-1]), flush=True)
    test_rust_parity.ensure_sources_unchanged(sources, tested_sources)
    report = {"schema": 1, "minecraft": lock["minecraft"]["version"], "data_version": lab.version_info()["world_version"],
              "platform": platform.platform(), "java": run(["java", "--version"]).splitlines()[0],
              "rustc": run(["rustc", "--version"]).strip(), "references": references,
              "sources_sha256": tested_sources,
              "notes": ["Synthetic NBT and region fixtures serialized and verified by pinned vanilla classes.",
                        "Includes all four region codecs, external oversized chunks, UTF-16 edge cases, mixed lists, palette mutation, and the full exported block-state ID registry.",
                        "Storage-only edits; no world generation, simulation, lighting, live multiplayer, renderer, FPS or whole-game parity claim."]}
    lab.write_json(report_path, report)
    lab.write_json(base / "report.json", report)
    print(f"World-data interoperability passed: {report_path}")


if __name__ == "__main__":
    main()
