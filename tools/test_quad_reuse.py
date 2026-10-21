#!/usr/bin/env python3
"""Compare actual terrain quad geometry and allocation with discarded-quad reuse."""

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
    fixture = build_mod.build_integration_fixture("quad-reuse", "dev.horizonlab.dispatch.QuadReuseIntegration")
    mod = root / "dist" / f"horizon-{build_pack.read_lock()['pack_version']}.jar"
    original_sha256 = hashlib.sha256(mod.read_bytes()).hexdigest()
    results = []
    for enabled in [False, True]:
        instance = root / ".local/runtime" / ("quad-reuse-fixture-patched" if enabled else "quad-reuse-fixture-upstream")
        build_pack.prepare_instance("m2-balanced", instance, mod)
        if hashlib.sha256((instance / "mods" / mod.name).read_bytes()).hexdigest() != original_sha256:
            raise AssertionError("Fixture runtime mod does not match the intended build")
        shutil.copy2(fixture, instance / "mods/quad-reuse-fixture.jar")
        (instance / "config/horizon-quad-reuse.properties").write_text(f"enabled={str(enabled).lower()}\n")
        command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance), "--heap", "3G", "--compact-headers"]
        if platform.system() == "Linux" and not args.native_display:
            command = ["xvfb-run", "-a", *command]
        environment = os.environ.copy()
        environment["_JAVA_OPTIONS"] = "-Dhorizon.fixture.exit=true"
        run = run_fixture(command, instance, environment)
        marker = "HORIZON_QUAD_RESULT "
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        if run.returncode or len(reports) != 1:
            raise RuntimeError(f"Quad reuse fixture failed; inspect {instance / 'fixture.log'}")
        report = reports[0]
        if report["enabled"] != enabled or report["input_faces"] != 55296 or report["measured_faces"] != 3072000 or report["thread_workers"] != 4 or not report["applied_hook_checked"]:
            raise AssertionError("Applied quad reuse fixture did not complete its geometry checks")
        results.append(report)
        print(json.dumps(report), flush=True)
    for key in ["geometry_digest", "geometry_quads", "input_faces", "checksum", "thread_geometry_digest"]:
        if results[0][key] != results[1][key]:
            raise AssertionError(f"Quad reuse changed upstream geometry: {key}")
    output = root / "dist/quad-reuse-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "distant_horizons": "3.3.4", "original_mod_sha256": original_sha256, "results": results,
        "notes": ["Actual publisher quad builder inside Fabric with generated faces and lighting/material values.",
                  "Opaque/translucent geometry, pooled error flags and every retained quad field match before and after upstream merging.",
                  "Fresh allocations reuse at most one discarded successful-premerge quad; existing pool selection stays upstream.",
                  "Three sequential warmed caller-loop observations are not independent forks; compilation can vary.",
                  "Allocation reduction is not retained RAM, game FPS or a Mac GPU benchmark."]}, indent=2) + "\n")
    print(f"Applied terrain quad reuse checks passed: {output}")


if __name__ == "__main__":
    main()
