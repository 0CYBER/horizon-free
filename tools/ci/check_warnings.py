#!/usr/bin/env python3
"""Run Cargo checks to completion with lint warnings, then reject workspace warnings or compiler failures.

Usage: python3 tools/ci/check_warnings.py cargo clippy --keep-going --workspace --all-targets
Cargo's JSON diagnostics include cached warnings from fresh crates. Render them normally, but enforce workspace
members' severity here rather than in rustc: a denied lint otherwise prevents metadata for dependent crates.
Foreign dependency diagnostics remain visible but are not counted, matching the gate's previous warning scope.
"""
import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor


def forward_diagnostics(stream, structured: bool, members: set[str]) -> int:
    """Render one Cargo stream and count warnings; keep stderr separate so it cannot corrupt JSON lines."""
    warnings = 0
    for line in stream:
        message = None
        if structured:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                pass
        if isinstance(message, dict) and message.get("reason") == "compiler-message":
            diagnostic = message["message"]
            warnings += diagnostic["level"] == "warning" and message["package_id"] in members
            print(diagnostic.get("rendered") or diagnostic["message"] + "\n", end="", flush=True)
        elif not isinstance(message, dict) or "reason" not in message:
            print(line, end="", flush=True)
            # Cargo/build-script warnings were never denied by rustc's -D warnings. Keep them visible only.
    return warnings


def main(command: list[str]) -> int:
    # Cargo flags precede the rustc/clippy argument separator. The gate owns this command, not untrusted input.
    split = command.index("--") if "--" in command else len(command)
    subcommand = next(index for index, arg in enumerate(command[:split]) if arg in ("clippy", "check"))
    metadata = [*command[:subcommand], "metadata", "--no-deps", "--format-version=1"]
    for index, arg in enumerate(command[:split]):
        if arg == "--manifest-path":
            metadata.extend([arg, command[index + 1]])
        elif arg.startswith("--manifest-path=") or arg in ("--locked", "--offline", "--frozen"):
            metadata.append(arg)
    listing = subprocess.run(metadata, capture_output=True, text=True)
    print(listing.stderr, end="", file=sys.stderr, flush=True)
    if listing.returncode:
        return listing.returncode
    members = set(json.loads(listing.stdout)["workspace_members"])
    clippy = command[subcommand] == "clippy"
    command = [*command[:split], "--message-format=json-diagnostic-rendered-ansi", *command[split:]]
    env = dict(os.environ)
    # Workspace crates have explicit deny/forbid lints. Cap those too, without changing clippy dependencies'
    # RUSTFLAGS fingerprints/cache keys. Clippy forwards arguments after -- only to workspace members.
    if clippy:
        if "--" not in command:
            command.append("--")
        command.append("--cap-lints=warn")
    elif "CARGO_ENCODED_RUSTFLAGS" in env:
        flags = env["CARGO_ENCODED_RUSTFLAGS"]
        env["CARGO_ENCODED_RUSTFLAGS"] = (flags + "\x1f" if flags else "") + "--cap-lints=warn"
    else:
        # SDK cargo-check steps already use their own RUSTFLAGS and target directory.
        env["RUSTFLAGS"] = env.get("RUSTFLAGS", "") + " --cap-lints=warn"
    with subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, errors="replace") as process, ThreadPoolExecutor(1) as pool:
        stderr = pool.submit(forward_diagnostics, process.stderr, False, members)
        warnings = forward_diagnostics(process.stdout, True, members) + stderr.result()
        code = process.wait()
    if warnings:
        print(f"::error::{warnings} workspace compiler warning records emitted; warnings fail this check after all crates finish",
              flush=True)
    return code if code else int(bool(warnings))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
