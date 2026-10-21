#!/usr/bin/env python3
"""Exercise applied DH dispatch hooks in isolated Fabric clients; no world or account needed."""

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

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


def failure_summary(run, report_count):
    """Publish only exit/count and bounded Java type names, never log messages."""
    types = []
    pattern = r"(?:Caused by:|Exception in thread \"[^\"]*\")\s+([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)+)(?=[:\s]|$)"
    for kind in re.findall(pattern, run.stdout, flags=re.ASCII):
        if len(kind) <= 200 and kind not in types:
            types.append(kind)
        if len(types) == 8:
            break
    return {"exit_code": run.returncode, "result_count": report_count, "java_exception_types": types}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-display", action="store_true", help="Use the existing display instead of Linux Xvfb")
    args = parser.parse_args()
    root = build_pack.ROOT
    fixture = build_mod.build_dispatch_fixture()
    version = build_pack.read_lock()["pack_version"]
    mod = root / "dist" / f"horizon-{version}.jar"
    results = []
    for enabled in [False, True]:
        instance = root / ".local/runtime" / ("dispatch-fixture-patched" if enabled else "dispatch-fixture-upstream")
        build_pack.prepare_instance("dispatch-retry", instance, mod)
        shutil.copy2(fixture, instance / "mods/dispatch-fixture.jar")
        (instance / "config/horizon-dispatch-retry.properties").write_text(f"enabled={str(enabled).lower()}\n")
        command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance),
                   "--heap", "3G", "--compact-headers"]
        if platform.system() == "Linux" and not args.native_display:
            command = ["xvfb-run", "-a", *command]
        environment = os.environ.copy()
        environment["_JAVA_OPTIONS"] = "-Dhorizon.fixture.exit=true"
        try:
            run = run_fixture(command, instance, environment)
        except RuntimeError:
            print("HORIZON_FIXTURE_FAILURE " + json.dumps({"fixture": "dispatch", "completed": False}), flush=True)
            raise RuntimeError("Dispatch fixture did not complete; full diagnostics remain private") from None
        marker = "HORIZON_DISPATCH_RESULT "
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        if run.returncode or len(reports) != 1:
            print("HORIZON_FIXTURE_FAILURE " + json.dumps(failure_summary(run, len(reports))), flush=True)
            raise RuntimeError("Dispatch fixture failed; full diagnostics remain in the private fixture.log")
        report = reports[0]
        if report["enabled"] != enabled or report["tasks_completed"] != 2 or report["occupied_slots"] != 0:
            raise AssertionError("Fixture outcome did not match the selected hook")
        results.append(report)
        print(json.dumps(report), flush=True)
    output = root / "dist/dispatch-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "distant_horizons": "3.3.4", "results": results,
        "notes": ["Synthetic full physical worker queue; both modes use the actual publisher dispatcher inside Fabric.",
                  "Allocation and CPU measure the submission caller only; retry scheduler activity is excluded.",
                  "Upstream keeps the caller blocked for the requested 200 ms observation window; patched returns earlier.",
                  "Both modes verify exact task counts, recovered occupied slots and fixture worker shutdown.",
                  "Not a game FPS, terrain completion, whole-game RAM, or hardware compatibility benchmark."]}, indent=2) + "\n")
    print(f"Applied-hook integration checks passed: {output}")


if __name__ == "__main__":
    main()
