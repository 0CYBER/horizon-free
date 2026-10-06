#!/usr/bin/env python3
"""Compare the Rust log sampler with both pinned vanilla JARs, optionally for every float."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess

try:
    from . import lab, test_rust_parity
except ImportError:
    import lab
    import test_rust_parity


def compare(reference, candidate, start, count):
    offset = 0
    with reference.open("rb") as expected, candidate.open("rb") as actual:
        while True:
            left, right = expected.read(1 << 20), actual.read(1 << 20)
            if len(left) != len(right):
                raise ValueError(f"log output length differs at byte {offset}")
            if left != right:
                index = next(i for i, (a, b) in enumerate(zip(left, right)) if a != b)
                at = index // 4 * 4
                bits = start + (offset + at) // 4
                raise ValueError(f"log mismatch for float bits {bits:08x}: vanilla {left[at:at+4].hex()}, Rust {right[at:at+4].hex()}")
            if not left:
                break
            offset += len(left)
    if offset != count * 4:
        raise ValueError(f"log output length {offset} does not cover {count} floats")
    return count


def ranges(exhaustive):
    if exhaustive:
        return [(0, 1 << 32)]
    return [(start, 1 << 16) for start in [0, 0x00800000, 0x3eff0000,
        0x3f7f0000, 0x3f800000, 0x40000000, 0x7f7f0000, 0x7f800000,
        0x7fff0000, 0x80000000, 0x80800000, 0xbf7f0000, 0xbf800000,
        0xff7f0000, 0xff800000, 0xffff0000]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exhaustive", action="store_true", help="check all 4,294,967,296 float bit patterns")
    args = parser.parse_args()
    build = lab.LOCAL / "rust-float-log-parity"
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / "report.json"
    report_path.unlink(missing_ok=True)
    lock, jars, libraries = test_rust_parity.verify_artifacts()
    source = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFloatLogOracle.java"
    sources = [source, Path(__file__), lab.ROOT / "tools/test_rust_parity.py", lab.ROOT / "tools/lab.py",
        *sorted((lab.ROOT / "crates/horizon-core").rglob("*.rs")), lab.ROOT / "crates/horizon-core/Cargo.toml",
        lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock"]
    tested_sources = test_rust_parity.source_hashes(sources)
    classes = build / "classes"
    classes.mkdir(exist_ok=True)
    dependency_path = os.pathsep.join(map(str, libraries))
    run = test_rust_parity.run
    run(["javac", "--release", str(lock["minecraft"]["java_major"]), "-proc:none", "-cp",
        os.pathsep.join([str(jars["server"]), dependency_path]), "-d", str(classes), str(source)])
    run(["cargo", "build", "--release", "--bin", "horizon-float-log-parity"])
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=lab.ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "release" / ("horizon-float-log-parity.exe" if os.name == "nt" else "horizon-float-log-parity")
    comparisons = []
    for start, count in ranges(args.exhaustive):
        candidate = build / "rust.bin"
        with candidate.open("wb") as output:
            run([str(binary), str(start), str(count)], stdout=output)
        candidate_digest = lab.digest(candidate, "sha256")
        references = []
        for side, jar in jars.items():
            reference = build / f"vanilla-{side}.bin"
            with reference.open("wb") as output:
                run(["java", "-Xmx512M", "-cp", os.pathsep.join([str(classes), str(jar), dependency_path]),
                    "dev.horizonlab.parity.VanillaFloatLogOracle", str(start), str(count)], stdout=output)
            matched = compare(reference, candidate, start, count)
            references.append({"side": side, "jar_sha256": lab.digest(jar, "sha256"),
                # Complete byte equality above proves the same content digest;
                # avoid rereading another 16 GiB per exhaustive reference.
                "output_sha256": candidate_digest, "matched_float_values": matched})
            print(f"{side}: {matched:,} float log values match Rust from bits {start:08x}", flush=True)
        comparisons.append({"start_bits": start, "count": count, "rust_output_sha256": candidate_digest, "references": references})
    test_rust_parity.ensure_sources_unchanged(sources, tested_sources)
    report = {"schema": 1, "minecraft": lock["minecraft"]["version"], "platform": platform.platform(),
        "java": subprocess.check_output(["java", "--version"], text=True).splitlines()[0],
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "manifest_sha1": lock["minecraft"]["manifest_sha1"], "exhaustive": args.exhaustive,
        "sources_sha256": tested_sources, "comparisons": comparisons,
        "scope": "Compiled scalar UnaryFunction.LogSampler compared by float result bits across the recorded input bit ranges, including signed zeros, subnormals, finite values, infinities and NaNs. NaN results are canonicalized as Float.floatToIntBits. Exhaustive mode checks all 2^32 float input bit patterns against both checksum-pinned JARs on this machine's JDK. Bulk composition, range propagation, compiler rewriting and other operations belong to the separate density suite. This does not establish double-precision Math.log parity, other-platform math parity, complete terrain, gameplay or performance."}
    lab.write_json(report_path, report)
    print(f"Float log parity passed: {report_path}", flush=True)


if __name__ == "__main__":
    main()
