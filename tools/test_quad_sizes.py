#!/usr/bin/env python3
"""Measure shallow quad-reuse retention in private Fabric clients, not whole-game RAM."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import zipfile

try:
    from . import lab
except ImportError:
    import lab

try:
    from . import build_mod, build_pack
    from .test_culling_rays import run_fixture
except ImportError:
    import build_mod
    import build_pack
    from test_culling_rays import run_fixture


def build_agent():
    output = build_mod.BUILD / "size-agent"
    classes = output / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    source = build_mod.SOURCE / "test/java/dev/horizonlab/metrics/ObjectSizeAgent.java"
    subprocess.run(["javac", "--release", "25", "-d", str(classes), str(source)], check=True)
    agent = output / "size-agent.jar"
    with zipfile.ZipFile(agent, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\n"
                         "Premain-Class: dev.horizonlab.metrics.ObjectSizeAgent\n\n")
        for path in sorted(classes.rglob("*.class")):
            archive.write(path, path.relative_to(classes).as_posix())
    return agent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-display", action="store_true")
    args = parser.parse_args()
    root = build_pack.ROOT
    agent = build_agent()
    fixture = build_mod.build_integration_fixture("quad-size", "dev.horizonlab.dispatch.QuadSizeIntegration")
    mod = root / "dist" / f"horizon-{build_pack.read_lock()['pack_version']}.jar"
    mod_hash = hashlib.sha256(mod.read_bytes()).hexdigest()
    results = []
    for headers in [False, True]:
        for enabled in [False, True]:
            instance = root / ".local/runtime" / f"quad-size-{headers}-{enabled}"
            build_pack.prepare_instance("m2-balanced", instance, mod)
            assert hashlib.sha256((instance / "mods" / mod.name).read_bytes()).hexdigest() == mod_hash
            shutil.copy2(fixture, instance / "mods/quad-size-fixture.jar")
            (instance / "config/horizon-quad-reuse.properties").write_text(f"enabled={str(enabled).lower()}\n")
            environment = os.environ.copy()
            header_option = "-XX:" + ("+" if headers else "-") + "UseCompactObjectHeaders"
            if '"' in str(agent) or "\n" in str(agent):
                raise ValueError("Instrumentation agent path contains an unsupported quote or newline")
            environment["_JAVA_OPTIONS"] = f'-javaagent:"{agent}" {header_option} -Dhorizon.fixture.exit=true'
            command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance), "--heap", "3G"]
            if platform.system() == "Linux" and not args.native_display:
                command = ["xvfb-run", "-a", *command]
            run = run_fixture(command, instance, environment)
            marker = "HORIZON_SIZE_RESULT "
            reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
            if run.returncode or len(reports) != 1:
                raise RuntimeError("Private quad-size fixture failed; diagnostics remain in its fixture.log")
            report = reports[0]
            if report["enabled"] != enabled or report["compact_headers"] != headers or report["geometry_quads"] != 1:
                raise AssertionError("Size probe did not measure the requested hook/header combination")
            if bool(report["pending_scratch_shallow_bytes"]) != enabled:
                raise AssertionError("Size probe retained an unexpected scratch object")
            results.append(report)
            print(json.dumps(report), flush=True)
    differences = []
    for control, candidate in zip(results[::2], results[1::2]):
        differences.append({"compact_headers": candidate["compact_headers"],
                            "additional_builder_shallow_bytes": candidate["builder_shallow_bytes"] - control["builder_shallow_bytes"],
                            "additional_builder_and_scratch_bytes": candidate["builder_shallow_bytes"] - control["builder_shallow_bytes"] + candidate["pending_scratch_shallow_bytes"]})
    output = root / "dist/quad-size-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "distant_horizons": "3.3.4", "original_mod_sha256": mod_hash,
        "results": results, "differences": differences,
        "notes": ["Instrumentation.getObjectSize measures shallow object sizes inside real applied Fabric clients.",
                  "Four adjacent north-facing quads premerge to one retained geometry quad in each combination.",
                  "Builder plus pending scratch difference excludes referenced lists, existing pools and every other game object.",
                  "Object alignment/header/compressed-reference settings can differ on other JVMs or architectures.",
                  "Not deep heap, GC residency, whole-game RSS/unified RAM, FPS or Apple Silicon performance.",
                  "The agent and fixture are private test binaries excluded from playable packs."]}, indent=2) + "\n")
    print(f"Quad object-size observations: {output}")


if __name__ == "__main__":
    main()
