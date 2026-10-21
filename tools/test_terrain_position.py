#!/usr/bin/env python3
"""Compare the actual full-data lookup with original and equivalent position arithmetic."""

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


try:
    from .test_culling_rays import run_fixture
except ImportError:
    from test_culling_rays import run_fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-display", action="store_true")
    args = parser.parse_args()
    root = build_pack.ROOT
    fixture = build_mod.build_integration_fixture("terrain-position", "dev.horizonlab.dispatch.PositionIntegration")
    mod = root / "dist" / f"horizon-{build_pack.read_lock()['pack_version']}.jar"
    original_sha256 = hashlib.sha256(mod.read_bytes()).hexdigest()
    results = []
    for enabled in [False, True]:
        instance = root / ".local/runtime" / ("terrain-position-fixture-patched" if enabled else "terrain-position-fixture-upstream")
        build_pack.prepare_instance("m2-balanced", instance, mod)
        if hashlib.sha256((instance / "mods" / mod.name).read_bytes()).hexdigest() != original_sha256:
            raise AssertionError("Fixture runtime mod does not match the intended build")
        shutil.copy2(fixture, instance / "mods/terrain-position-fixture.jar")
        (instance / "config/horizon-terrain-position.properties").write_text(f"enabled={str(enabled).lower()}\n")
        command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance), "--heap", "3G", "--compact-headers"]
        if platform.system() == "Linux" and not args.native_display:
            command = ["xvfb-run", "-a", *command]
        environment = os.environ.copy()
        environment["_JAVA_OPTIONS"] = "-Dhorizon.fixture.exit=true"
        run = run_fixture(command, instance, environment)
        marker = "HORIZON_POSITION_RESULT "
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        if run.returncode or len(reports) != 1:
            raise RuntimeError(f"Terrain position fixture failed; inspect {instance / 'fixture.log'}")
        report = reports[0]
        if report["enabled"] != enabled or report["owners"] != 30 or report["queries"] != 491520 or not report["data_unchanged"] or not report["applied_hook_checked"]:
            raise AssertionError("Applied position fixture did not complete its lookup checks")
        results.append(report)
        print(json.dumps(report), flush=True)
    for key in ["lookup_digest", "data_fingerprint", "nonzero", "checksum"]:
        if results[0][key] != results[1][key]:
            raise AssertionError(f"Position redirect changed upstream lookup: {key}")
    output = root / "dist/terrain-position-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "distant_horizons": "3.3.4", "original_mod_sha256": original_sha256, "results": results,
        "notes": ["Actual public full-data lookup inside Fabric with generated numeric column data.",
                  "Thirty owners span positive/negative origins and six detail levels; output digests and source fingerprints match.",
                  "The actual redirect method is checked in each mode; source column data remains unchanged.",
                  "Three sequential warmed caller-loop observations are not independent forks; JIT compilation can vary.",
                  "Not whole-game FPS/RAM, rendered-pixel equivalence or a Mac GPU benchmark."]}, indent=2) + "\n")
    print(f"Applied terrain position checks passed: {output}")


if __name__ == "__main__":
    main()
