#!/usr/bin/env python3
"""Exercise TNT's transient transfer state through the actual isolated Game loaders.

The contracts follow pinned PrimedTnt.teleport and its save/load methods. This
runs no Java server, world generation or cross-dimensional travel transcript.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def child_env(scratch, resources=None):
    env = dict(os.environ)
    env.pop("HORIZON_TEST_RESOURCES", None)
    for key, child in [("HORIZON_HOME", "home"), ("HORIZON_LAUNCHER_HOME", "launcher"),
                       ("XDG_DATA_HOME", "xdg-data"), ("XDG_CONFIG_HOME", "xdg-config"),
                       ("HORIZON_LAUNCHER_TRASH", "trash"),
                       ("HORIZON_LAUNCHER_CLIPBOARD", "clipboard"),
                       ("HORIZON_LAUNCHER_DESKTOP", "desktop")]:
        env[key] = str(scratch / child)
    env = lab.game_env(env)
    if resources is not None:
        env["HORIZON_TEST_RESOURCES"] = str(resources)
    return env


def main():
    resources = Path(os.environ.get("HORIZON_TEST_RESOURCES", lab.RESOURCES)).resolve()
    if not (resources / "movement-properties.nbt").exists():
        raise RuntimeError("Set HORIZON_TEST_RESOURCES to prepared pinned resources")
    build = lab.LOCAL / "tnt-portal-state-parity"
    build.mkdir(parents=True, exist_ok=True)
    report = build / "report.json"
    report.unlink(missing_ok=True)
    sources = [Path(__file__), lab.ROOT / "research-lock.json", lab.ROOT / "Cargo.toml",
               lab.ROOT / "Cargo.lock", lab.ROOT / "tools/lab.py",
               *sorted((lab.ROOT / "crates/horizon-client/src").rglob("*.rs")),
               *sorted((lab.ROOT / "crates/horizon-client-render/src").rglob("*.rs"))]
    hashes = base.source_hashes(sources)
    with tempfile.TemporaryDirectory(prefix="horizon-tnt-state-build-") as directory:
        command = ["cargo", "test", "--locked", "-p", "horizon-client", "--lib",
                   "--no-run", "--message-format=json"]
        with (build / "compile.jsonl").open("w", encoding="utf-8") as output:
            result = subprocess.run(command, cwd=lab.ROOT,
                                    env=child_env(Path(directory)), stdout=output)
    artifacts = []
    for line in (build / "compile.jsonl").read_text(encoding="utf-8").splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if result.returncode and row.get("reason") == "compiler-message":
            print(row.get("message", {}).get("rendered", ""), file=sys.stderr, end="")
        if row.get("reason") == "compiler-artifact" and row.get("executable") \
                and row.get("target", {}).get("name") == "horizon_client" \
                and row.get("profile", {}).get("test"):
            artifacts.append(Path(row["executable"]))
    result.check_returncode()
    if len(artifacts) != 1:
        raise AssertionError("Expected one actual gameplay library test executable")
    binary = artifacts[0]
    binary_hash = lab.digest(binary, "sha256")
    with tempfile.TemporaryDirectory(prefix="horizon-tnt-state-game-") as directory:
        scratch = Path(directory)
        with (build / "rust.log").open("w", encoding="utf-8") as log:
            subprocess.run([str(binary), "portal_transfer_", "--include-ignored", "--color", "never"],
                           cwd=scratch, env=child_env(scratch, resources), check=True,
                           stdout=log, stderr=subprocess.STDOUT)
    output = (build / "rust.log").read_text(encoding="utf-8")
    if "running 3 tests" not in output or "3 passed; 0 failed" not in output:
        raise AssertionError("All three transfer-state regressions must execute")
    if binary_hash != lab.digest(binary, "sha256"):
        raise RuntimeError("Test executable changed during validation")
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, {"minecraft": lab.VERSION, "sources_sha256": hashes,
                           "rust_binary_sha256": binary_hash,
                           "rust_log_sha256": lab.digest(build / "rust.log", "sha256"),
                           "rust_tests": 3, "scope": __doc__})
    print("TNT transfer state: three actual Game contracts passed")


if __name__ == "__main__":
    main()
