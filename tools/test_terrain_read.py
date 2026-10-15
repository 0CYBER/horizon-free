#!/usr/bin/env python3
"""Exercise interruptible blending reads on an actual transformed Minecraft IOWorker."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys

try:
    from . import lab
except ImportError:
    import lab

try:
    from . import build_mod, build_pack
except ImportError:
    import build_mod
    import build_pack


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-display", action="store_true")
    args = parser.parse_args()
    root = build_pack.ROOT
    fixture = build_mod.build_integration_fixture("terrain-read", "dev.horizonlab.dispatch.TerrainReadIntegration")
    mod = root / "dist" / f"horizon-{build_pack.read_lock()['pack_version']}.jar"
    original_sha256 = hashlib.sha256(mod.read_bytes()).hexdigest()
    results = []
    for enabled in [False, True]:
        instance = root / ".local/runtime" / ("terrain-read-fixture-patched" if enabled else "terrain-read-fixture-upstream")
        build_pack.prepare_instance("m2-balanced", instance, mod)
        if hashlib.sha256((instance / "mods" / mod.name).read_bytes()).hexdigest() != original_sha256:
            raise AssertionError("Runtime mod does not match the intended build")
        shutil.copy2(fixture, instance / "mods/terrain-read-fixture.jar")
        (instance / "config/horizon-terrain-read-wait.properties").write_text(f"enabled={str(enabled).lower()}\n")
        command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance), "--heap", "3G", "--compact-headers"]
        if platform.system() == "Linux" and not args.native_display:
            command = ["xvfb-run", "-a", *command]
        environment = os.environ.copy()
        environment["_JAVA_OPTIONS"] = "-Dhorizon.fixture.exit=true"
        run = subprocess.run(command, cwd=root, env=environment, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, timeout=240)
        (instance / "fixture.log").write_text(run.stdout)
        marker = "HORIZON_TERRAIN_READ_RESULT "
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        if run.returncode or len(reports) != 1:
            raise RuntimeError(f"Terrain read fixture failed; inspect {instance / 'fixture.log'}")
        report = reports[0]
        if report["enabled"] != enabled or not all(report.get(key) for key in [
                "applied_io_worker_checked", "ordinary_join_checked", "shared_future_preserved",
                "exception_identity_checked", "completed_interrupt_checked", "cancelled_completion_checked", "preinterrupted_pending_checked"]):
            raise AssertionError("Applied terrain read hook did not preserve the required outcomes")
        results.append(report)
        print(json.dumps(report), flush=True)
    output = root / "dist/terrain-read-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "distant_horizons": "3.3.4", "original_mod_sha256": original_sha256, "results": results,
        "notes": ["Actual game IOWorker under Fabric, with controlled cached blending-read futures.",
                  "DH worker interruption releases only its waiter; the shared future remains consumable.",
                  "Ordinary callers retain join behavior; completed values, cancellation and exception identity are checked.",
                  "This is a shutdown correctness fixture, not an FPS or world-generation throughput benchmark."]}, indent=2) + "\n")
    print(f"Applied terrain read checks passed: {output}")


if __name__ == "__main__":
    main()
