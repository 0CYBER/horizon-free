#!/usr/bin/env python3
"""The crates a pull request's `cargo test` must cover, for GITHUB_OUTPUT.

Prints `packages=-p a -p b ...`, or `packages=` when the whole workspace must run (main and nightly never call this).
A package is selected when a changed file is in its directory, when it depends (normal, dev or build) on a selected one,
or when it reads the changed file at run time from outside its own directory (READERS: tests read other crates' files).
A change outside every package directory, such as Cargo.lock, .github/ or tools/, selects the whole workspace.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools.ci.rust_changes import changed_paths  # noqa: E402

# Files and folders a package reads from outside its own directory (tests at run time, build scripts and
# include_str!/include_bytes! at compile time). A changed path equal to or under an entry selects the package, as if its
# own files had changed: whatever depends on it is selected too.
# tests/test_ci_test_scope.py scans crates/ and xtask/ and fails on a read missing here. Entries outside every package
# directory only document the read: a change there selects the whole workspace.
HORIZON_CLIENT = [
    # The build script's generator and native_hooks_tests read the hook targets' sources and the catalog.
    "crates/horizon-client-render/src", "crates/horizon-world/src", "crates/horizon-items/src",
    "docs/api/launch-reference.md", "docs/horizon-ctl-commands.md", "docs/parity/random-chance-seeded.jsonl",
    "native-hooks.json", "tools/native-hook-fragments", "packaging/windows/exe.rs", "research-lock.json",
    "tests/fixtures",
]
MOD_API_GEN = [
    "native-hooks.json", "tools/native-hook-fragments",
    "crates/horizon-client/src", "crates/horizon-client-render/src", "crates/horizon-world/src",
]
READERS = {
    "cargo-horizon-mod": ["crates/horizon-mods/tests/index"],
    "horizon-client": HORIZON_CLIENT,
    # Shares horizon-client's sources, so its unit tests run the same reads.
    "horizon-client-lib": HORIZON_CLIENT,
    # lang.rs embeds the bundled language files.
    "horizon-client-core": ["crates/horizon-client/assets/horizon/lang"],
    "horizon-client-render": [
        "crates/horizon-client/assets/horizon/lang", "crates/horizon-client/assets/horizon/textures/gui",
    ],
    "horizon-mod-api-gen": MOD_API_GEN,
    "horizon-mod-api-gen-tests": MOD_API_GEN,
    "horizon-official": ["packaging/windows/exe.rs"],
    "horizon-perf": ["crates/horizon-perf-format/tests/fixtures", "tests/fixtures/pacing_slow_frames.hzperf"],
    "horizon-port-pack": ["docs/modpacks/atm10-mods.csv", "native-hooks.json"],
    "horizon-prepare": [
        "crates/horizon-client/src/game/launch_api.rs", "mods/horizon/src/test/java", "research-lock.json",
    ],
    "horizon-release-sign": ["tools/ci/sign_native_build.sh", "tools/ci/sign_release.sh"],
    "horizon-server": ["crates/horizon-theme/fonts", "crates/horizon-theme/icons", "docs/api/server-reference.md"],
    "horizon-shader-api": ["docs/api/shaders-reference.md", "docs/api/shaders-reference-0.2.md"],
    "horizon-world": ["crates/horizon-core", "Cargo.lock"],
    # Commands (tokens, icons, package), not tests: listed so the scan finds nothing unexplained.
    "xtask": [
        "crates/horizon-perf/web", "crates/horizon-prepare", "crates/horizon-theme", "docs/media/brand",
        "mods/horizon/src/test/java", "native-hooks.json", "packaging", "research-lock.json", "tools",
    ],
}
# What a package's tests only read at run time and nothing else uses: the package is selected, its dependents are not.
# horizon-client-core's lang.rs tests read the sources of every crate for the language keys the code names.
SCANNED = {"horizon-client-core": ["crates"]}
# Literals the scan takes for a walk up to crates/ that are not one (a ".." among path-traversal checks).
NOT_READS = {"horizon-client": {"crates"}, "horizon-client-lib": {"crates"}, "horizon-prepare": {"crates"}}


def under(path, folder):
    return path == folder or path.startswith(folder + "/")


class Workspace:
    def __init__(self, metadata):
        root = metadata["workspace_root"]

        def relative(path):
            return os.path.relpath(os.path.normpath(path), root)

        names = [package["name"] for package in metadata["packages"]]
        home = {p["name"]: relative(os.path.dirname(p["manifest_path"])) for p in metadata["packages"]}
        self.dirs = {name: {home[name]} for name in names}
        for package in metadata["packages"]:
            for target in package["targets"]:
                source = relative(target["src_path"])
                # horizon-client-lib builds horizon-client's files: all of that package's folder counts.
                folder = max((d for d in home.values() if under(source, d)), key=len, default=None)
                self.dirs[package["name"]].add(folder or os.path.dirname(source))
        self.home = home
        self.dependents = {name: set() for name in names}
        for package in metadata["packages"]:
            for dependency in package["dependencies"]:
                if dependency["name"] in self.dependents:
                    self.dependents[dependency["name"]].add(package["name"])

    def owners(self, path):
        return {name for name, dirs in self.dirs.items() if any(under(path, d) for d in dirs)}

    def readers(self, path, table=None):
        table = READERS if table is None else table
        return {name for name, paths in table.items() if name in self.dirs and any(under(path, p) for p in paths)}

    def scope(self, paths):
        """The packages to test, or None for the whole workspace."""
        chosen, scanners = set(), set()
        for path in paths:
            chosen |= self.readers(path)
            scanners |= self.readers(path, SCANNED)
            owners = self.owners(path)
            chosen |= owners
            # Documentation nobody builds from, unless a test reads it (READERS) or a crate owns it.
            inert = (path.startswith("docs/") or path.endswith(".md")) and not path.startswith(".github/")
            if not owners and not inert:
                return None
        todo = list(chosen)
        while todo:
            for dependent in self.dependents[todo.pop()] - chosen:
                chosen.add(dependent)
                todo.append(dependent)
        chosen |= scanners
        return None if chosen >= set(self.dirs) else chosen


def flags(chosen):
    return "packages=" + " ".join(f"-p {name}" for name in sorted(chosen or ()))


def cargo_metadata():
    return json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"]))


LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
ROOT_FILES = {"native-hooks.json", "research-lock.json", "Cargo.lock"}
ROOT_JOIN = re.compile(r"^/?(\.\./?)+$")


def reads(workspace):
    """{package: {path: where}}: tracked paths outside the package that its sources name as string literals.

    A literal is resolved against the package folder (run time, CARGO_MANIFEST_DIR) and its file's folder (include_str!).
    "../.." in a file with CARGO_MANIFEST_DIR is the repository root or crates/, reported as "." or "crates".
    """
    tracked = subprocess.check_output(["git", "ls-files", "-z"]).decode().split("\0")
    known = {"."} | {t for path in tracked if path for t in [path, *map(str, Path(path).parents)]}
    found = {}
    for file in tracked:
        owners = workspace.owners(file) if file.endswith(".rs") else ()
        if not owners:
            continue
        text = Path(file).read_text(errors="replace")
        manifest = "CARGO_MANIFEST_DIR" in text
        for name in owners:
            home = workspace.home[name]
            for literal in set(LITERAL.findall(text)):
                stripped = literal.strip("/")
                if re.search(r"[\s{}]|://", literal):
                    continue
                if ROOT_JOIN.match(literal):
                    # A walk up from the manifest folder: the repository root, or crates/ for its siblings.
                    candidates = {os.path.normpath(os.path.join(home, stripped))} & {".", "crates"} if manifest else set()
                    candidates = {c for c in candidates if c != home}
                elif stripped.strip("./"):
                    candidates = {os.path.normpath(os.path.join(base, stripped)) for base in (home, os.path.dirname(file))}
                    if "/" in literal and not literal.startswith(".") or literal in ROOT_FILES:
                        candidates.add(os.path.normpath(stripped))
                    candidates = {c for c in candidates if c in known and not c.startswith("..")
                                  and not any(under(c, d) or under(d, c) for d in workspace.dirs[name])}
                else:
                    continue
                for candidate in candidates:
                    found.setdefault(name, {})[candidate] = f"{file}: {literal}"
    return found


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    args = parser.parse_args()
    chosen = Workspace(cargo_metadata()).scope(changed_paths(args.base, args.head))
    print(flags(chosen))
