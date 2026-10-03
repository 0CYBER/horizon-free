#!/usr/bin/env python3
"""Compile only original unobfuscated-26.3 mod sources; never bundle game classes."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

try:
    from . import lab, runtime
except ImportError:
    import lab
    import runtime

ROOT = lab.ROOT
SOURCE = ROOT / "mods/horizon-lab/src"
BUILD = ROOT / ".local/mod-build"


def test_metrics():
    destination = BUILD / "metric-tests"
    destination.mkdir(parents=True, exist_ok=True)
    sources = [SOURCE / "main/java/dev/horizonlab/metrics" / name
               for name in ["SampleStats.java", "Recording.java"]]
    sources.append(SOURCE / "test/java/dev/horizonlab/metrics/MetricsTest.java")
    subprocess.run(["javac", "--release", "25", "-d", str(destination), *map(str, sources)], check=True)
    subprocess.run(["java", "-cp", str(destination), "dev.horizonlab.metrics.MetricsTest"], check=True)


def build():
    lock = json.loads((ROOT / "packs/mods.lock.json").read_text())
    _, jars, _ = runtime.prepare_client(assets=False)
    api = lock["projects"]["fabric-api"]
    api_jar = runtime.fetch(api["url"], ROOT / ".local/pack-cache" / api["filename"],
                            api["hashes"]["sha512"], "sha512")
    nested = BUILD / "fabric-api"
    nested.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(api_jar) as archive:
        for name in archive.namelist():
            if name.startswith("META-INF/jars/") and name.endswith(".jar"):
                destination = nested / Path(name).name
                destination.write_bytes(archive.read(name))
                jars.append(destination)
    classes = BUILD / "classes"
    if classes.exists():
        shutil.rmtree(classes)
    classes.mkdir(parents=True)
    sources = sorted((SOURCE / "main/java").rglob("*.java"))
    command = ["--release", "25", "-proc:none", "-encoding", "UTF-8", "-d", str(classes),
               "-classpath", os.pathsep.join(map(str, jars)), *map(str, sources)]
    argfile = BUILD / "javac.args"
    argfile.write_text("\n".join(json.dumps(arg) for arg in command) + "\n")
    subprocess.run(["javac", "@" + str(argfile)], cwd=ROOT, check=True)
    output = ROOT / "dist" / f"horizon-lab-{lock['pack_version']}.jar"
    output.parent.mkdir(exist_ok=True)
    entries = {path.relative_to(classes).as_posix(): path.read_bytes() for path in classes.rglob("*.class")}
    if any(not name.startswith("dev/horizonlab/") for name in entries):
        raise ValueError("Only original Horizon Lab classes may be packaged")
    resources = SOURCE / "main/resources"
    entries.update({path.relative_to(resources).as_posix(): path.read_bytes() for path in resources.rglob("*") if path.is_file()})
    entries["LICENSE"] = (ROOT / "LICENSE").read_bytes()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name in sorted(entries):
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, entries[name])
    print(f"Built {output} ({output.stat().st_size} bytes, sha256 {lab.digest(output, 'sha256')})")
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-only", action="store_true")
    args = parser.parse_args()
    test_metrics()
    if not args.test_only:
        build()
