#!/usr/bin/env python3
"""Check actual DH tick transitions and allocation with the optional membership hook off/on."""

import argparse
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
    fixture = build_mod.build_tick_dedup_fixture()
    mod = root / "dist" / f"horizon-{build_pack.read_lock()['pack_version']}.jar"
    results = []
    for enabled in [False, True]:
        instance = root / ".local/runtime" / ("tick-dedup-fixture-patched" if enabled else "tick-dedup-fixture-upstream")
        build_pack.prepare_instance("m2-balanced", instance, mod)
        shutil.copy2(fixture, instance / "mods/tick-dedup-fixture.jar")
        (instance / "config/horizon-tick-dedup.properties").write_text(f"enabled={str(enabled).lower()}\n")
        command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance), "--heap", "3G", "--compact-headers"]
        if platform.system() == "Linux" and not args.native_display:
            command = ["xvfb-run", "-a", *command]
        environment = os.environ.copy()
        environment["_JAVA_OPTIONS"] = "-Dhorizon.fixture.exit=true"
        run = subprocess.run(command, cwd=root, env=environment, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, timeout=240)
        (instance / "fixture.log").write_text(run.stdout)
        marker = "HORIZON_TICK_DEDUP_RESULT "
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        if run.returncode or len(reports) != 1:
            raise RuntimeError(f"Tick dedup fixture failed; inspect {instance / 'fixture.log'}")
        report = reports[0]
        if report["enabled"] != enabled or report["verified_transitions"] != 6000 or report["checksum"] != 10240000:
            raise AssertionError("Applied hook or semantic outcome did not match")
        results.append(report)
        print(json.dumps(report), flush=True)
    if results[0]["allocated_bytes"] <= 0 or results[1]["allocated_bytes"] >= results[0]["allocated_bytes"] / 20:
        raise AssertionError("Membership hook did not remove repeated entry allocation")
    output = root / "dist/tick-dedup-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "distant_horizons": "3.3.4", "results": results,
        "notes": ["Actual publisher tick holder under Fabric, warmed with preallocated nodes and a reused action list.",
                  "Checks duplicate/null membership, enable/disable precedence, published iteration order and clear boundaries.",
                  "Only private membership checks change; action collections and traversal remain upstream.",
                  "CPU and allocation measure the calling thread in one isolated loop; no statistical speed claim.",
                  "Not a game FPS, visible terrain completion, whole-game RAM or Mac GPU benchmark."]}, indent=2) + "\n")
    print(f"Applied tick membership checks passed: {output}")


if __name__ == "__main__":
    main()
