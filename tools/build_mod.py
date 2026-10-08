#!/usr/bin/env python3
"""Compile only original unobfuscated-26.3 mod sources; never bundle game classes."""

import argparse
import json
import os
import platform
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
SOURCE = ROOT / "mods/horizon/src"
BUILD = ROOT / ".local/mod-build"


def test_metrics():
    destination = BUILD / "metric-tests"
    destination.mkdir(parents=True, exist_ok=True)
    sources = [SOURCE / "main/java/dev/horizonlab/metrics" / name
               for name in ["SampleStats.java", "Recording.java", "ProcessMemoryMonitor.java", "TickState.java", "ServerContext.java"]]
    sources.append(SOURCE / "test/java/dev/horizonlab/metrics/MetricsTest.java")
    subprocess.run(["javac", "--release", "25", "-d", str(destination), *map(str, sources)], check=True)
    subprocess.run(["java", "-cp", str(destination), "dev.horizonlab.metrics.MetricsTest"], check=True)
    subprocess.run(["java", "-XX:+UseCompactObjectHeaders", "-cp", str(destination),
                    "dev.horizonlab.metrics.MetricsTest"], check=True)
    subprocess.run(["java", "-Xms32M", "-Xmx256M", "-XX:+UseZGC", "-XX:+UseCompactObjectHeaders", "-cp", str(destination),
                    "dev.horizonlab.metrics.MetricsTest"], check=True)
    budget_sources = [SOURCE / "main/java/dev/horizonlab/budget/TerrainBudget.java",
                      SOURCE / "test/java/dev/horizonlab/budget/TerrainBudgetTest.java"]
    subprocess.run(["javac", "--release", "25", "-d", str(destination), *map(str, budget_sources)], check=True)
    subprocess.run(["java", "-cp", str(destination), "dev.horizonlab.budget.TerrainBudgetTest"], check=True)
    worker_sources = [SOURCE / "main/java/dev/horizonlab/budget/WorkerPacer.java",
                      SOURCE / "test/java/dev/horizonlab/budget/WorkerPacerTest.java"]
    subprocess.run(["javac", "--release", "25", "-d", str(destination), *map(str, worker_sources)], check=True)
    subprocess.run(["java", "-cp", str(destination), "dev.horizonlab.budget.WorkerPacerTest"], check=True)
    debounce_sources = [SOURCE / "main/java/dev/horizonlab/budget/DebounceWait.java",
                        SOURCE / "test/java/dev/horizonlab/budget/DebounceWaitTest.java"]
    subprocess.run(["javac", "--release", "25", "-d", str(destination), *map(str, debounce_sources)], check=True)
    subprocess.run(["java", "-cp", str(destination), "dev.horizonlab.budget.DebounceWaitTest"], check=True)
    idle_sources = [SOURCE / "main/java/dev/horizonlab/budget/WakeableIdleWait.java",
                    SOURCE / "test/java/dev/horizonlab/budget/WakeableIdleWaitTest.java"]
    subprocess.run(["javac", "--release", "25", "-d", str(destination), *map(str, idle_sources)], check=True)
    subprocess.run(["java", "-cp", str(destination), "dev.horizonlab.budget.WakeableIdleWaitTest"], check=True)
    dispatch_sources = [SOURCE / "main/java/dev/horizonlab/budget/DispatchRetry.java",
                        SOURCE / "test/java/dev/horizonlab/budget/DispatchRetryTest.java"]
    subprocess.run(["javac", "--release", "25", "-d", str(destination), *map(str, dispatch_sources)], check=True)
    subprocess.run(["java", "-cp", str(destination), "dev.horizonlab.budget.DispatchRetryTest"], check=True)


def build():
    lock = json.loads((ROOT / "packs/mods.lock.json").read_text())
    _, jars, _ = runtime.prepare_client(assets=False)
    with zipfile.ZipFile(jars[0]) as game_archive:
        game_version = json.loads(game_archive.read("version.json"))
    if game_version["world_version"] != lock["options_data_version"]:
        raise ValueError("Pack options data version does not match the verified game JAR")
    dh = lock["projects"]["distanthorizons"]
    jars.append(runtime.fetch(dh["url"], ROOT / ".local/pack-cache" / dh["filename"],
                              dh["hashes"]["sha512"], "sha512"))
    culling = lock["projects"]["entityculling"]
    jars.append(runtime.fetch(culling["url"], ROOT / ".local/pack-cache" / culling["filename"],
                              culling["hashes"]["sha512"], "sha512"))
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
    adapter_tests = BUILD / "adapter-tests"
    adapter_tests.mkdir(parents=True, exist_ok=True)
    test_classpath = os.pathsep.join(map(str, [classes, *jars]))
    subprocess.run(["javac", "--release", "25", "-proc:none", "-classpath", test_classpath,
                    "-d", str(adapter_tests),
                    str(SOURCE / "test/java/dev/horizonlab/client/DistantTerrainBudgetTest.java"),
                    str(SOURCE / "test/java/dev/horizonlab/client/DistantTerrainSettingsTest.java"),
                    str(SOURCE / "test/java/dev/horizonlab/geometry/RelativeTerrainPositionTest.java"),
                    str(SOURCE / "test/java/dev/horizonlab/metrics/ResidentMemoryTest.java")], check=True)
    subprocess.run(["java", "-cp", os.pathsep.join([str(adapter_tests), test_classpath]),
                    "dev.horizonlab.client.DistantTerrainBudgetTest"], check=True)
    subprocess.run(["java", "-cp", os.pathsep.join([str(adapter_tests), test_classpath]),
                    "dev.horizonlab.client.DistantTerrainSettingsTest"], check=True)
    subprocess.run(["java", "--enable-native-access=ALL-UNNAMED", "-cp", os.pathsep.join([str(adapter_tests), test_classpath]),
                    "dev.horizonlab.metrics.ResidentMemoryTest"], check=True)
    subprocess.run(["java", "-cp", os.pathsep.join([str(adapter_tests), test_classpath]),
                    "dev.horizonlab.geometry.RelativeTerrainPositionTest"], check=True)
    output = ROOT / "dist" / f"horizon-{lock['pack_version']}.jar"
    output.parent.mkdir(exist_ok=True)
    entries = {path.relative_to(classes).as_posix(): path.read_bytes() for path in classes.rglob("*.class")}
    if any(not name.startswith("dev/horizonlab/") for name in entries):
        raise ValueError("Only original Horizon classes may be packaged")
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


def build_dispatch_fixture():
    return build_integration_fixture("dispatch", "dev.horizonlab.dispatch.DispatchIntegration")


def build_tick_dedup_fixture():
    return build_integration_fixture("tick-dedup", "dev.horizonlab.dispatch.TickDedupIntegration")


def build_culling_empty_fixture():
    return build_integration_fixture("culling-empty", "dev.horizonlab.dispatch.CullingEmptyIntegration", ["entityculling"])


def build_integration_fixture(name, entrypoint, extra_projects=(), fixture_mixins=(), fixture_dependencies=None):
    """Create a private integration-test mod; never include it in release packs."""
    arguments = [json.loads(line) for line in (BUILD / "javac.args").read_text().splitlines()]
    classpath = arguments[arguments.index("-classpath") + 1] + os.pathsep + str(BUILD / "classes")
    lock = json.loads((ROOT / "packs/mods.lock.json").read_text())
    for name_project in extra_projects:
        project = lock["projects"][name_project]
        jar = runtime.fetch(project["url"], ROOT / ".local/pack-cache" / project["filename"],
                            project["hashes"]["sha512"], "sha512")
        classpath += os.pathsep + str(jar)
    classes = BUILD / f"{name}-fixture"
    if classes.exists():
        shutil.rmtree(classes)
    classes.mkdir(parents=True)
    fixture_sources = [SOURCE / "test/java" / (entrypoint.replace(".", "/") + ".java")]
    fixture_sources.extend(SOURCE / "test/java/dev/horizonlab/dispatch/fixture" / (mixin + ".java") for mixin in fixture_mixins)
    subprocess.run(["javac", "--release", "25", "-proc:none", "-classpath", classpath,
                    "-d", str(classes), *map(str, fixture_sources)], check=True)
    output = BUILD / f"{name}-fixture.jar"
    metadata = {"schemaVersion": 1, "id": "horizon_" + name.replace("-", "_") + "_fixture",
                "version": "1.0.0", "environment": "client", "entrypoints": {"client": [entrypoint]},
                "depends": {"horizonlab": "*", **({"distanthorizons": "3.3.4"}
                    if fixture_dependencies is None else fixture_dependencies)}}
    if fixture_mixins:
        metadata["mixins"] = [name + "-fixture.mixins.json"]
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        if fixture_mixins:
            archive.writestr(metadata["mixins"][0], json.dumps({"required": True, "package": "dev.horizonlab.dispatch.fixture",
                "compatibilityLevel": "JAVA_25", "client": list(fixture_mixins), "injectors": {"defaultRequire": 1}}))
        for path in sorted(classes.rglob("*.class")):
            archive.write(path, path.relative_to(classes).as_posix())
        archive.writestr("fabric.mod.json", json.dumps(metadata))
    print(f"Private {name} integration fixture: {output}")
    return output


def benchmark_workers():
    arguments = [json.loads(line) for line in (BUILD / "javac.args").read_text().splitlines()]
    classpath = os.pathsep.join([str(BUILD / "classes"), arguments[arguments.index("-classpath") + 1]])
    tests = BUILD / "worker-benchmark"
    tests.mkdir(parents=True, exist_ok=True)
    subprocess.run(["javac", "--release", "25", "-proc:none", "-cp", classpath,
                    "-d", str(tests), str(SOURCE / "test/java/dev/horizonlab/budget/WorkerPacingBenchmark.java")], check=True)
    output = ROOT / "dist/worker-pacing-benchmark.txt"
    with output.open("w", encoding="utf-8") as report:
        report.write(f"Java {subprocess.check_output(['java', '--version'], text=True).splitlines()[0]}\n")
        report.write(f"Platform {platform.system()} {platform.release()} {platform.machine()}; logical CPUs {os.cpu_count()}\n")
        report.flush()
        for repeat in range(3):
            for ratio in ["0.5", "0.05"]:
                for mode in ["upstream", "precise"]:
                    subprocess.run(["java", "-cp", os.pathsep.join([str(tests), classpath]),
                                    "dev.horizonlab.budget.WorkerPacingBenchmark", ratio, "10000", "50000", mode],
                                   stdout=report, check=True)
    print(output.read_text(), end="")


def benchmark_debounce():
    arguments = [json.loads(line) for line in (BUILD / "javac.args").read_text().splitlines()]
    classpath = os.pathsep.join([str(BUILD / "classes"), arguments[arguments.index("-classpath") + 1]])
    tests = BUILD / "debounce-benchmark"
    tests.mkdir(parents=True, exist_ok=True)
    subprocess.run(["javac", "--release", "25", "-proc:none", "-cp", classpath,
                    "-d", str(tests), str(SOURCE / "test/java/dev/horizonlab/budget/DebounceQueueBenchmark.java")], check=True)
    output = ROOT / "dist/debounce-queue-benchmark.txt"
    with output.open("w", encoding="utf-8") as report:
        report.write(f"Java {subprocess.check_output(['java', '--version'], text=True).splitlines()[0]}\n")
        report.write(f"Platform {platform.system()} {platform.release()} {platform.machine()}; logical CPUs {os.cpu_count()}\n")
        report.flush()
        subprocess.run(["java", "-cp", os.pathsep.join([str(tests), classpath]),
                        "dev.horizonlab.budget.DebounceQueueBenchmark"], stdout=report, check=True)
    print(output.read_text(), end="")


def benchmark_headers():
    arguments = [json.loads(line) for line in (BUILD / "javac.args").read_text().splitlines()]
    classpath = arguments[arguments.index("-classpath") + 1]
    tests = BUILD / "header-benchmark"
    tests.mkdir(parents=True, exist_ok=True)
    subprocess.run(["javac", "--release", "25", "-proc:none", "-cp", classpath,
                    "-d", str(tests), str(SOURCE / "test/java/dev/horizonlab/metrics/ObjectHeaderBenchmark.java")], check=True)
    output = ROOT / "dist/object-header-benchmark.txt"
    with output.open("w", encoding="utf-8") as report:
        report.write(f"Java {subprocess.check_output(['java', '--version'], text=True).splitlines()[0]}\n")
        report.write(f"Platform {platform.system()} {platform.release()} {platform.machine()}; logical CPUs {os.cpu_count()}\n")
        report.flush()
        for repeat in range(3):
            for enabled in [False, True]:
                report.write(f"fork={repeat} requested_compact_headers={enabled}\n")
                report.flush()
                subprocess.run(["java", "-Xms128M", "-Xmx512M",
                                "-XX:" + ("+" if enabled else "-") + "UseCompactObjectHeaders",
                                "-cp", os.pathsep.join([str(tests), classpath]),
                                "dev.horizonlab.metrics.ObjectHeaderBenchmark", str(enabled).lower()],
                               stdout=report, check=True)
    print(output.read_text(), end="")


def benchmark_idle():
    arguments = [json.loads(line) for line in (BUILD / "javac.args").read_text().splitlines()]
    classpath = os.pathsep.join([str(BUILD / "classes"), arguments[arguments.index("-classpath") + 1]])
    tests = BUILD / "idle-benchmark"
    tests.mkdir(parents=True, exist_ok=True)
    subprocess.run(["javac", "--release", "25", "-proc:none", "-cp", classpath,
                    "-d", str(tests), str(SOURCE / "test/java/dev/horizonlab/budget/IdleQueueBenchmark.java")], check=True)
    output = ROOT / "dist/idle-queue-benchmark.txt"
    with output.open("w", encoding="utf-8") as report:
        report.write(f"Java {subprocess.check_output(['java', '--version'], text=True).splitlines()[0]}\n")
        report.write(f"Platform {platform.system()} {platform.release()} {platform.machine()}; logical CPUs {os.cpu_count()}\n")
        report.flush()
        subprocess.run(["java", "-cp", os.pathsep.join([str(tests), classpath]),
                        "dev.horizonlab.budget.IdleQueueBenchmark"], stdout=report, check=True)
    print(output.read_text(), end="")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-only", action="store_true")
    parser.add_argument("--benchmark-workers", action="store_true", help="Measure tiny-job executor pacing; not Minecraft FPS")
    parser.add_argument("--benchmark-headers", action="store_true", help="Measure object allocation with compact headers off/on; not game RAM")
    args = parser.parse_args()
    if args.test_only and (args.benchmark_workers or args.benchmark_headers):
        parser.error("Benchmarks require the compiled mod; omit --test-only")
    test_metrics()
    if not args.test_only:
        build()
        if args.benchmark_workers:
            benchmark_workers()
        if args.benchmark_headers:
            benchmark_headers()
