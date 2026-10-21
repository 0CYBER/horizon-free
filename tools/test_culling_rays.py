#!/usr/bin/env python3
"""Compare actual visibility geometry and per-instance numeric scratch ownership."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import signal
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


class FixtureTimeout(RuntimeError):
    """Keep timeout output private while allowing selected caller diagnostics."""
    def __init__(self, run, private_log):
        super().__init__(f"Private fixture timed out; inspect {private_log}")
        self.run = run


def run_fixture(command, instance, environment):
    # Keep all children of this private fixture in a controllable process group.
    process = subprocess.Popen(command, cwd=build_pack.ROOT, env=environment, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               start_new_session=platform.system() != "Windows")
    def stop_tree(force=False):
        if platform.system() == "Windows":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True, check=False)
        else:
            try: os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
            except ProcessLookupError: pass
    try:
        output, _ = process.communicate(timeout=240)
    except subprocess.TimeoutExpired:
        stop_tree()
        try: output, _ = process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            stop_tree(True); output, _ = process.communicate(timeout=10)
        (instance / "fixture.log").write_text(output)
        raise FixtureTimeout(subprocess.CompletedProcess(command, process.returncode, output),
                             instance / 'fixture.log') from None
    (instance / "fixture.log").write_text(output)
    return subprocess.CompletedProcess(command, process.returncode, output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-display", action="store_true")
    args = parser.parse_args()
    root = build_pack.ROOT
    fixture = build_mod.build_integration_fixture("culling-rays", "dev.horizonlab.dispatch.CullingRayIntegration", ["entityculling"], ["CullingRayAccess"])
    mod = root / "dist" / f"horizon-{build_pack.read_lock()['pack_version']}.jar"
    original_sha256 = hashlib.sha256(mod.read_bytes()).hexdigest()
    results = []
    for enabled in [False, True]:
        instance = root / ".local/runtime" / ("culling-rays-fixture-patched" if enabled else "culling-rays-fixture-upstream")
        build_pack.prepare_instance("m2-balanced", instance, mod)
        if hashlib.sha256((instance / "mods" / mod.name).read_bytes()).hexdigest() != original_sha256:
            raise AssertionError("Fixture runtime mod does not match the intended build")
        shutil.copy2(fixture, instance / "mods/culling-rays-fixture.jar")
        (instance / "config/horizon-culling-rays.properties").write_text(f"enabled={str(enabled).lower()}\n")
        command = [sys.executable, str(root / "tools/runtime.py"), "demo", "--instance", str(instance), "--heap", "3G", "--compact-headers"]
        if platform.system() == "Linux" and not args.native_display:
            command = ["xvfb-run", "-a", *command]
        environment = os.environ.copy()
        environment["_JAVA_OPTIONS"] = "-Dhorizon.fixture.exit=true"
        run = run_fixture(command, instance, environment)
        marker = "HORIZON_CULLING_RAYS_RESULT "
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        if run.returncode or len(reports) != 1:
            raise RuntimeError(f"Culling scratch fixture failed; inspect {instance / 'fixture.log'}")
        report = reports[0]
        if report["enabled"] != enabled or report["public_boxes"] != 12800 or report["edge_rays"] != 2197 or not report["input_unchanged"] or not report["scratch_ownership_checked"]:
            raise AssertionError("Applied culling scratch fixture did not complete its geometry checks")
        results.append(report)
        print(json.dumps(report), flush=True)
    for key in ["visibility_digest", "query_digest", "intersection_digest", "provider_reads", "checksum"]:
        if results[0][key] != results[1][key]:
            raise AssertionError(f"Scratch redirect changed upstream geometry: {key}")
    output = root / "dist/culling-rays-integration.json"
    output.write_text(json.dumps({"schema": 1, "platform": platform.platform(), "architecture": platform.machine(),
        "minecraft": lab.VERSION, "entity_culling": "1.11.2", "original_mod_sha256": original_sha256, "results": results,
        "notes": ["Actual upstream culling methods inside Fabric; private fixture invokers do not enter production packs.",
                  "Two interleaved instances and several occluder patterns; matching boolean and query digests.",
                  "Signed zero, tiny/large values, infinities and NaN are exercised directly in ray intersection.",
                  "Unchanged input vectors, independent instance scratch and upstream arithmetic values are checked.",
                  "Allocation and CPU are single warmed caller-loop observations; JIT folding can vary.",
                  "Not a game FPS, visual-equivalence proof, whole-game RAM or Mac GPU benchmark."]}, indent=2) + "\n")
    print(f"Applied culling scratch checks passed: {output}")


if __name__ == "__main__":
    main()
