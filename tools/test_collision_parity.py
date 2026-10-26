#!/usr/bin/env python3
"""Validate registry-backed cached collisions against the pinned vanilla binaries."""
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


def counts(output):
    marker = "HORIZON_COLLISION_RESULT "
    found = [json.loads(line.split(marker, 1)[1]) for line in output.splitlines() if marker in line]
    if len(found) != 1:
        raise AssertionError(f"Missing collision counts: {found}")
    result = found[0]
    if result["registry_states"] != 35723 or result["cached_states"] + result["uncached_states"] != result["registry_states"] or result["queries"] != result["cached_states"] * 6 or result["unique_shapes"] <= 0:
        raise AssertionError(f"Incomplete collision coverage: {result}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    directory = lab.LOCAL / "rust-collision-parity"
    directory.mkdir(parents=True, exist_ok=True)
    report_path = directory / "report.json"
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = test_rust_parity.verify_artifacts()
    for earlier in directory.glob("run-*"):  # one run per run used to stay behind: 5 GB here after a few weeks
        shutil.rmtree(earlier, ignore_errors=True)
    base = Path(tempfile.mkdtemp(prefix="run-", dir=directory))
    oracle = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java"
    sources = [oracle, Path(__file__).resolve(), lab.ROOT / "tools/test_rust_parity.py", lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock", lab.ROOT / "crates/horizon-core/Cargo.toml", lab.ROOT / "crates/horizon-world/Cargo.toml", *sorted((lab.ROOT / "crates/horizon-core/src").rglob("*.rs")), *sorted((lab.ROOT / "crates/horizon-world/src").rglob("*.rs")), lab.ROOT / "crates/horizon-world/examples/probe/collision_fixture.rs"]
    tested_sources = test_rust_parity.source_hashes(sources)
    classes = base / "classes"
    classes.mkdir()
    dependency_path = os.pathsep.join(map(str, libraries))
    test_rust_parity.run(["javac", "--release", "25", "-proc:none", "-cp", os.pathsep.join([str(jars["server"]), dependency_path]), "-d", str(classes), str(oracle)])
    def java(side, mode):
        result = subprocess.run(["java", "-Xmx1G", "-cp", os.pathsep.join([str(classes), str(jars[side]), dependency_path]), "dev.horizonlab.parity.VanillaCollisionOracle", mode, str(base)], cwd=lab.ROOT, capture_output=True, text=True, timeout=300)
        (base / f"{side}-{mode}.log").write_text(result.stdout + result.stderr, encoding="utf-8")
        result.check_returncode()
        return result.stdout
    expected = counts(java("server", "generate"))
    test_rust_parity.run(["cargo", "build", "--release", "-p", "horizon-world", "--example", "world_probe"])
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=lab.ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "release/examples" / ("world_probe.exe" if os.name=="nt" else "world_probe")
    actual = counts(subprocess.check_output([str(binary), "collision_fixture", str(base)], cwd=lab.ROOT, text=True))
    if actual != expected:
        raise AssertionError(f"Collision coverage differs: vanilla={expected}, Rust={actual}")
    canonical = {"empty_states":0,"block_states":0,"other_cached_states":0}
    for line in (base / "rust.txt").read_text().splitlines():
        if line == 'canonical true false': canonical['empty_states'] += 1
        elif line == 'canonical false true': canonical['block_states'] += 1
        elif line == 'canonical false false': canonical['other_cached_states'] += 1
    if sum(canonical.values()) != expected['cached_states']: raise AssertionError('incomplete canonical identity comparison')
    references = []
    for side, jar in jars.items():
        java(side, side)
        reference = base / f"vanilla-{side}.txt"
        matched = test_rust_parity.compare(reference, base / "rust.txt")
        references.append({"side": side, "jar_sha256": lab.digest(jar, "sha256"), "output_sha256": lab.digest(reference, "sha256"), "matched_output_lines": matched, **expected})
        print(f"{side}: {matched:,} cached collision output lines match", flush=True)
    test_rust_parity.ensure_sources_unchanged(sources, tested_sources)
    report = {"schema": 1, "minecraft": lock["minecraft"]["version"], "manifest_sha1": lock["minecraft"]["manifest_sha1"], "platform": platform.platform(),
              "java": subprocess.check_output(["java", "--version"], text=True).splitlines()[0], "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
              "inputs_sha256": lab.digest(base / "inputs.txt", "sha256"), "rust_output_sha256": lab.digest(base / "rust.txt", "sha256"), "sources_sha256": tested_sources, "references": references, "canonical_identities":canonical,
              "scope": "Pinned no-context cached collision branch only. Registry identity validation, resident section lookup, grids, native canonical empty/block identities distinct from equal geometry, discrete extrema including empty shapes, greedy boxes, bounds, translated collide, ray hits, closest points and moved cell lookup. Uncached and entity-context behavior have separate comparison suites."}
    lab.write_json(report_path, report)
    lab.write_json(base / "report.json", report)
    print(f"Cached collision comparison passed: {report_path}")


if __name__ == "__main__":
    main()
