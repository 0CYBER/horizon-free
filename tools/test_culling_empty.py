#!/usr/bin/env python3
"""Check live culling whitelist registration and the optional empty-iterator shortcut."""

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
    fixture = build_mod.build_culling_empty_fixture()
    mod = root / "dist" / f"horizon-{build_pack.read_lock()['pack_version']}.jar"
    results = []
    for enabled in [False, True]:
        instance = root / ".local/runtime" / ("culling-empty-fixture-patched" if enabled else "culling-empty-fixture-upstream")
        build_pack.prepare_instance("m2-balanced", instance, mod)
        shutil.copy2(fixture, instance / "mods/culling-empty-fixture.jar")
        (instance / "config/horizon-culling-empty.properties").write_text(f"enabled={str(enabled).lower()}\n")
        command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance), "--heap", "3G", "--compact-headers"]
        if platform.system() == "Linux" and not args.native_display:
            command = ["xvfb-run", "-a", *command]
        environment = os.environ.copy()
        environment["_JAVA_OPTIONS"] = "-Dhorizon.fixture.exit=true"
        run = subprocess.run(command, cwd=root, env=environment, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, timeout=240)
        (instance / "fixture.log").write_text(run.stdout)
        marker = "HORIZON_CULLING_EMPTY_RESULT "
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        if run.returncode or len(reports) != 1:
            raise RuntimeError(f"Culling empty fixture failed; inspect {instance / 'fixture.log'}")
        report = reports[0]
        if report["enabled"] != enabled or not report["live_registration_checked"] or not report["block_callback_order_checked"] or not report["type_guard_checked"] or report["result"]:
            raise AssertionError("Applied hook or semantic outcome did not match")
        results.append(report)
        print(json.dumps(report), flush=True)
    if [r["empty_iterator_calls"] for r in results] != [1, 0]:
        raise AssertionError("Empty shortcut did not apply to actual methods")
    output = root / "dist/culling-empty-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "entity_culling": "1.11.2", "results": results,
        "notes": ["Actual publisher whitelist methods inside Fabric; live registration and block-entity callbacks checked.",
                  "Nonempty iteration, callback order, short circuit, null/wrong-type inputs and clearing retain upstream behavior.",
                  "Empty-state iterator counts use a private instrumented set; allocation loop uses normal empty HashSets.",
                  "Entity callbacks with a valid game entity are not exercised by this menu-only fixture.",
                  "JIT escape analysis can remove allocations in an isolated loop; real culling-thread profiles are needed.",
                  "Not a game FPS, whole-game RAM, visual equivalence or Mac GPU benchmark."]}, indent=2) + "\n")
    print(f"Applied empty whitelist checks passed: {output}")


if __name__ == "__main__":
    main()
