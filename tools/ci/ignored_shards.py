#!/usr/bin/env python3
"""Print the ignored-tests shard matrix: every shard, or the ones a pull request needs.

Usage:
  ignored_shards.py --matrix --all      `matrix=`, every shard and the daily-rows legs (the daily run)
  ignored_shards.py --prepare PACKAGE [ARG...]   run what PACKAGE's tests need built first (PREPARE), with ARGs
                                                 appended (`-j2`); nothing when it needs nothing
  ignored_shards.py --matrix BASE HEAD  `matrix=`, the shards this pull request must run

The ignored-tests check is a matrix with one package per shard; this file is the single
place that names them. A shard is selected when a changed crate is its package or a
transitive dependency of it. Workspace-wide files, and any path no crate owns, select
every shard; pushes to main always run everything (the `changes` job passes --all).
crates/horizon-client holds horizon-client-lib's sources too (its [lib] path reaches
into that tree), so that directory counts for both packages; the launcher is its own
workspace (launcher/Cargo.lock) and selects nothing.
"""

import json
import subprocess
import sys
import tomllib
from functools import lru_cache
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools.ci import rust_changes  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]

# One package per shard, balanced on the measured test times of the single-job run this
# plan replaced (CI run #952, e8752e146f: horizon-client 380s, horizon-client-render
# 279s, horizon-client-lib 189s, 16m53s wall); horizon-world and horizon-items joined when every library
# test started running with the data, not only the #[ignore]d ones. Rebalance by editing this tuple only.
SHARDS = ("horizon-client", "horizon-client-lib", "horizon-client-render", "horizon-world", "horizon-items")
# The leg that runs the daily rows (tools/ci/ignored-tests/daily-*.txt) that no package shard above runs: the moved
# tests of every other package, and of the shard packages' targets their shard does not build (tools/ci/ignored_tests.py
# daily_rows_groups). Coverage, not speed: without it those tests would run nowhere. Main's full matrix only.
DAILY_ROWS = "daily-rows"
# The leg that runs each workspace's daily rows. The launcher's get a leg of their own, on the same runner size: its
# workspace builds GPUI and the launcher from nothing (25 minutes on 8 cores, measured cold), which does not fit beside
# the other packages' groups in one 60-minute leg. Coverage, not spreading a slow job.
DAILY_ROWS_LEGS = {".": DAILY_ROWS, "launcher": "daily-rows-launcher"}
# What a package's tests need built that `cargo test` does not build itself, by package: the one command, run
# before its tests by the daily run (tools/ci/ignored_tests.py), so they
# cannot drift. horizon-mod-package's holder tests start the `holder_actor` example from beside their test executable
# (target/debug/examples, the test profile's folder).
PREPARE = {
    "horizon-mod-package": ["cargo", "build", "--locked", "--profile", "test", "-p", "horizon-mod-package",
                            "--features", "runtime-process", "--example", "holder_actor"],
}
# A package too big for one test process's memory runs as several legs, `PACKAGE#NofM` (tools/ci/ignored_tests.py
# gives each test to exactly one leg). horizon-client's single process was killed (signal 9) on three main runs.
LEGS = {"horizon-client": 2}

# Files at the repository root that every package's build reads.
ROOT_WIDE_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}
# The crates directory whose sources build more than one of the shard packages.
SHARED_SOURCES = {"horizon-client": ("horizon-client", "horizon-client-lib")}
# Workspaces of their own: nothing under them builds the shard packages.
SEPARATE_WORKSPACES = ("launcher/",)
# Every Cargo workspace whose tests the daily rows may name, by folder: the game's, and the launcher's own (its own
# Cargo.lock; the daily-rows leg runs a launcher row from launcher/).
WORKSPACES = (".", "launcher")


def expand(packages):
    """The matrix entries for these shard packages: a package with legs becomes one entry per leg."""
    return [
        f"{package}#{number}of{LEGS[package]}" if package in LEGS else package
        for package in packages
        for number in range(1, LEGS.get(package, 1) + 1)
    ]


@lru_cache(maxsize=1)
def workspace_members():
    """{package: (its folder, its workspace's folder)} for every member of WORKSPACES, relative to the root."""
    members = {}
    for workspace in WORKSPACES:
        table = tomllib.loads((ROOT / workspace / "Cargo.toml").read_text())
        for member in ([workspace] if "package" in table else []) + [
                Path(workspace, folder).as_posix() for folder in table["workspace"].get("members", ())]:
            name = tomllib.loads((ROOT / member / "Cargo.toml").read_text())["package"]["name"]
            members[name] = (member, workspace)
    return members


def daily_rows_leg(package):
    """The leg that runs `package`'s daily rows that no package shard runs (DAILY_ROWS_LEGS, by its workspace)."""
    return DAILY_ROWS_LEGS[workspace_members()[package][1]]


def workspace_targets():
    """Each member's test targets as the daily rows name them, in every workspace (WORKSPACES):
    {package: {"lib", "bin:NAME", "test:NAME"}}.

    Read from the manifests and Cargo's default layout (src/lib.rs, src/main.rs, src/bin/, tests/), not by building."""
    targets = {}
    for member, _ in workspace_members().values():
        folder = ROOT / member
        table = tomllib.loads((folder / "Cargo.toml").read_text())
        name = table["package"]["name"]
        found = set()
        if "lib" in table or (folder / "src/lib.rs").is_file():
            found.add("lib")
        found.update(f"bin:{entry['name']}" for entry in table.get("bin", []))
        found.update(f"test:{entry['name']}" for entry in table.get("test", []))
        if table["package"].get("autobins", True):
            if (folder / "src/main.rs").is_file():
                found.add(f"bin:{name}")
            found.update(f"bin:{path.stem}" for path in (folder / "src/bin").glob("*.rs"))
            found.update(f"bin:{path.parent.name}" for path in (folder / "src/bin").glob("*/main.rs"))
        if table["package"].get("autotests", True):
            found.update(f"test:{path.stem}" for path in (folder / "tests").glob("*.rs"))
            found.update(f"test:{path.parent.name}" for path in (folder / "tests").glob("*/main.rs"))
        targets[name] = found
    return targets


def workspace_dependencies():
    """Each workspace member's dependency names (its manifest's dependencies and dev-dependencies)."""
    dependencies = {}
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        table = tomllib.loads(manifest.read_text())
        names = set()
        for section in ("dependencies", "dev-dependencies"):
            names.update(table.get(section, {}))
        dependencies[table["package"]["name"]] = names
    return dependencies


def depends_on(start, target, dependencies):
    """Whether `target` is `start` itself or a transitive dependency of `start`."""
    seen, todo = {start}, [start]
    while todo:
        current = todo.pop()
        if current == target:
            return True
        for name in dependencies.get(current, ()):
            if name not in seen:
                seen.add(name)
                todo.append(name)
    return False


def select(paths, dependencies):
    """The shard packages whose tests a changed path set can affect."""
    if not rust_changes.needs_rust(paths):
        return []
    owners = set()
    for path in paths:
        if not (path.startswith("docs/") or path.endswith(".md")):
            if any(path.startswith(prefix) for prefix in SEPARATE_WORKSPACES):
                continue
            if ("/" not in path and path in ROOT_WIDE_FILES) or path.startswith(".cargo/"):
                return list(SHARDS)
            if path.startswith("crates/") and path.count("/") >= 2:
                crate = path.split("/")[1]
                owners.update(SHARED_SOURCES.get(crate, (crate,)))
            else:
                return list(SHARDS)  # no crate owns it: run everything, as the single job did
    return [
        package for package in SHARDS
        if any(depends_on(package, owner, dependencies) for owner in owners)
    ]


if __name__ == "__main__":
    arguments = sys.argv[1:]
    if arguments[:1] == ["--prepare"] and len(arguments) >= 2:
        command = PREPARE.get(arguments[1])
        if command:
            print("$ " + " ".join([*command, *arguments[2:]]), flush=True)
            sys.exit(subprocess.run([*command, *arguments[2:]], cwd=ROOT).returncode)
        sys.exit(0)
    every = arguments[:2] == ["--matrix", "--all"]
    selected = len(arguments) == 3 and arguments[0] == "--matrix"
    if every:
        shards = [*expand(SHARDS), *DAILY_ROWS_LEGS.values()]
    elif selected:
        shards = expand(select(rust_changes.changed_paths(arguments[1], arguments[2]), workspace_dependencies()))
    else:
        sys.exit(__doc__)
    print("matrix=" + json.dumps({"shard": shards}))
