#!/usr/bin/env python3
"""Select CI checks from one fail-closed Git diff.

The default answers Rust selection; --launcher uses the launcher's dependency paths.
--all emits the shared Rust, launcher and tooling decisions in one answer.
server_build: needs_server_build's answer, which no job reads (see SERVER_BUILD_CHECK).
"""

import argparse
import subprocess
import sys
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools import release_train  # noqa: E402

# The root workspace's manifest and toolchain files: the launcher's path crates inherit from them.
ROOT_RUST = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}
# The check itself: a change to it runs it. install_rust.sh sets the job up (needs_rust already counts it: tools/ shell).
LAUNCHER_CHECK = {".github/workflows/ci.yml", "tools/ci/rust_changes.py", "tools/release_train.py",
                  "tools/ci/install_rust.sh"}


# The Python under tools/ and tests/: verify's Python steps and the parity jobs run it, and no Rust build or test reads
# it (Rust does read tools/ci/*.sh, so those still count). This script and the
# module it imports decide whether Rust runs at all, so a change to them runs it.
RUST_DECIDES = {"tools/ci/rust_changes.py", "tools/release_train.py"}


# Files needs_rust's rules would skip that Rust compiles in (include_str!, include_bytes!, include!), each with the
# Rust files that embed it: a change to one selects what a change to those files would. Kept exact by
# tests/test_ci_rust_changes.py, which finds every embed in the Rust sources.
EMBEDDED = {
    "crates/horizon-mod-api-gen/coverage-notes.md": ("crates/horizon-mod-api-gen/src/wire/host.rs",),
    "docs/media/nether.webp": ("launcher/src/glass_server_dialog.rs",),
    "docs/media/underwater.webp": ("launcher/src/glass_server_dialog.rs",),
    "docs/media/village.webp": ("launcher/src/glass_server_dialog.rs",),
    "docs/modpacks/atm10-mods.csv": ("crates/horizon-port-pack/src/classify.rs",),
    "docs/parity/random-chance-seeded.jsonl": (
        "crates/horizon-client/src/advancements/tests.rs",
        "crates/horizon-client/src/game/interface/advancements/mod.rs",
    ),
}


def with_embedders(paths):
    return [embedder for path in paths for embedder in (path, *EMBEDDED.get(path, ()))]


def needs_rust(paths):
    return needs_rust_alone(with_embedders(paths))


def needs_rust_alone(paths):
    """needs_rust without EMBEDDED."""
    return any(not (
        path.startswith("docs/") or path.endswith(".md")
        or path.startswith(("tools/", "tests/")) and path.endswith(".py") and path not in RUST_DECIDES
    ) for path in paths)


# verify's second clippy, the server build (horizon-client without default features): what it checks differently from
# the first is the feature-gated code of the client crates and the features the manifests choose. ci.yml does not read
# the answer: verify-server runs on every pull request that changes Rust, because a crate below the client can break
# the server build without touching either, and not in the merge queue.
SERVER_BUILD_CHECK = {".github/workflows/ci.yml", "tools/ci/rust_changes.py"}


def needs_server_build(paths):
    return any(path.startswith("crates/horizon-client") or path.endswith("Cargo.toml") or path in ROOT_RUST
               or path.startswith(".cargo/") or path in SERVER_BUILD_CHECK for path in paths)


def needs_launcher(paths):
    return any(path in ROOT_RUST or path in LAUNCHER_CHECK or (
        path.startswith(tuple(release_train.LAUNCHER_PATHS))
        and not path.startswith(tuple(release_train.LAUNCHER_IGNORED))
        and not path.endswith(".md")
    ) for path in with_embedders(paths))


WINDOWS_WASM_CHECK = {".github/workflows/ci.yml", "tools/ci/rust_changes.py", "tools/ci/rust-version",
                      "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}


def needs_windows_wasm_cache(paths):
    return any(path in WINDOWS_WASM_CHECK or path.startswith(("crates/horizon-mod-wasm/", ".cargo/"))
               for path in paths)


# Stands for "everything changed" when no diff can be computed: a root manifest every check and shard selector
# treats as touching all of them.
UNKNOWN_CHANGE = "Cargo.lock"


def needs_tooling(changes):
    paths = [path for _, path in changes]
    return needs_rust(paths) or any(
        path.startswith(("tools/", "tests/", ".github/", "docs/modding/", "roadmap/"))
        or path in {"README.md", "ROADMAP.md", "docs/README.md"}
        # Tests check linked target existence, even when they do not read its contents.
        or status != "M"
        for status, path in changes
    )


# What the mod API check (tools/check_mod_api_unchanged.py, verify's `mod API` part) reads: the game module and
# the generator's crates, the check itself and the surface report beside it, and what decides the set of crates it walks
# (manifests and the lockfile; the unknown-change stand-in is Cargo.lock). The generator also reads every `#[replaceable]` function in a lower crate, so a
# changed Rust file under crates/ that mentions the attribute, before or after, counts too (mentions_replaceable).
# horizon-game-discovery finds the native hook files the generator reads (hook_targets.rs).
MOD_API_PREFIXES = ("crates/horizon-client/src/game/", "crates/horizon-mod-", "crates/horizon-game-discovery/")
# The network NBT's tag ids and limits are in the core hash every wasm mod is checked by; the other core inputs live in
# crates/horizon-mod-*.
MOD_API_FILES = {"crates/horizon-client/src/game.rs", "crates/horizon-client/src/entry.rs",
                 "tools/check_mod_api_unchanged.py", "tools/mod_api_removed_internals.txt", "Cargo.lock",
                 ".github/workflows/ci.yml", "tools/ci/rust_changes.py", "tools/ci/surface_report.py",
                 "tools/ci/surface.toml", "crates/horizon-core/src/nbt.rs",
                 # The toolchain the generator parses with and the layouts are compiled by.
                 "rust-toolchain", "rust-toolchain.toml", ".cargo/config.toml"}
# The lower-crate internals a native mod may declare, one path per line (internals.json lists them): a file that
# holds one changes the API whether or not it mentions `replaceable`.
DECLARED_INTERNALS = "crates/horizon-mod-api-gen/internals.txt"
# Crates whose library is built from another crate's sources: horizon-client-lib's manifest points at
# horizon-client's (its root is src/client_lib.rs there).
SHARED_SOURCES = {"horizon_client_lib": "horizon_client"}


PATHS_PER_GREP = 200


def mentions_replaceable(base, head, paths):
    """Whether any of the Rust files `paths` has the word `replaceable` in `base` or in `head`. Fail closed: a git
    error counts as a mention."""
    for start in range(0, len(paths), PATHS_PER_GREP):
        chunk = paths[start:start + PATHS_PER_GREP]
        for revision in (base, head):
            found = subprocess.run(["git", "grep", "-q", "-e", "replaceable", revision, "--", *chunk],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
            if found.returncode != 1:
                return True
    return False


def declared_internals(*revisions):
    """The declarations of DECLARED_INTERNALS at each of `revisions`, or None when one can't be read (fail closed)."""
    declarations = set()
    for revision in revisions:
        shown = subprocess.run(["git", "show", f"{revision}:{DECLARED_INTERNALS}"], stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, check=False)
        if shown.returncode != 0:
            return None
        for line in shown.stdout.decode("utf-8", "replace").splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                declarations.add(line)
    return declarations


def module_of(path):
    """(crate, module path) of a Rust source file under crates/, or None: src/lib.rs (or another crate root) is the
    crate's root, an empty module; src/a/b.rs and src/a/b/mod.rs are `a::b`."""
    parts = path.split("/")
    if len(parts) < 4 or parts[0] != "crates" or parts[2] != "src" or not path.endswith(".rs"):
        return None
    krate, inside = parts[1].replace("-", "_"), parts[3:]
    if inside in (["lib.rs"], ["main.rs"], ["client_lib.rs"]):
        return krate, []
    if inside[-1] == "mod.rs":
        inside = inside[:-1]
    else:
        inside[-1] = inside[-1][:-len(".rs")]
    return krate, inside


def holds_declared(path, declarations):
    """Whether `path` may hold one of `declarations` (`crate::module::Item::member`): its crate's root, the module
    file of a module the declaration goes through, or a file of the declaring module's own folder (an `impl` in a
    submodule). Generic arguments (`Mesher<'_>`) are dropped first. An item a re-export moved elsewhere is missed."""
    module = module_of(path)
    if module is None:
        return False
    krate, inside = module
    for declaration in declarations:
        segments = [segment.split("<", 1)[0] for segment in declaration.split("::")]
        if SHARED_SOURCES.get(segments[0], segments[0]) != krate:
            continue
        reach = segments[1:-1]
        if inside == reach[:len(inside)]:
            return True
        # A submodule of the declaring module: the path's modules end where a type (capitalised) or the item starts.
        parent = inside[:-1]
        if parent and parent == reach[:len(parent)] and (len(parent) == len(reach) or reach[len(parent)][:1].isupper()):
            return True
    return False


def needs_mod_api(paths, mentions=lambda rust_files: False, declarations=frozenset()):
    """Whether a change can alter the generated mod API. `mentions` answers for the changed
    Rust files under crates/ that no listed path already covers; `declarations` are the declared lower-crate
    internals (declared_internals), None when they could not be read, which selects any Rust change."""
    if any(path.startswith(MOD_API_PREFIXES) and not path.endswith(".md") or path in MOD_API_FILES
           or path.startswith("crates/") and path.endswith("Cargo.toml") or path == "Cargo.toml" for path in paths):
        return True
    rust_files = [path for path in paths if path.startswith("crates/") and path.endswith(".rs")]
    if not rust_files:
        return False
    if declarations is None or any(holds_declared(path, declarations) for path in rust_files):
        return True
    return mentions(rust_files)


def selections(changes):
    paths = [path for _, path in changes]
    return {"rust_required": needs_rust(paths),
            "launcher_required": needs_launcher(paths),
            "tooling_required": needs_tooling(changes)}


def changed_entries(base, head):
    # Disable rename folding: retain both deleted and added paths, including source moved into docs.
    command = ["git", "diff", "--name-status", "--no-renames", "-z", f"{base}...{head}", "--"]
    for fetch_base in (False, True):
        # The event's base can be newer than the checked-out history (main moved on): fetch it once.
        if fetch_base:
            subprocess.run(["git", "fetch", "-q", "--no-tags", "origin", base], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        diff = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
        if diff.returncode == 0:
            fields = diff.stdout.decode("utf-8").split("\0")[:-1]
            return list(zip(fields[::2], fields[1::2]))
    # Never an error and never "nothing changed": assume everything did.
    print(f"rust_changes: no diff for {base}...{head}; treating everything as changed", file=sys.stderr)
    return [("?", UNKNOWN_CHANGE)]


def changed_paths(base, head):
    return [path for _, path in changed_entries(base, head)]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--launcher", action="store_true", help="answer for the launcher instead")
    mode.add_argument("--all", action="store_true", help="emit the Rust, launcher and tooling selections at once")
    args = parser.parse_args()
    changes = changed_entries(args.base, args.head)
    paths = [path for _, path in changes]
    if args.all:
        for name, required in selections(changes).items():
            print(f"{name}={str(required).lower()}")
    else:
        needs = needs_launcher if args.launcher else needs_rust
        print(f"required={str(needs(paths)).lower()}")
        print(f"server_build={str(needs_server_build(paths)).lower()}")
        if not args.launcher:
            mod_api = needs_mod_api(paths, lambda files: mentions_replaceable(args.base, args.head, files),
                                    declared_internals(args.base, args.head))
            print(f"mod_api={str(mod_api).lower()}")
            print(f"windows_wasm_cache={str(needs_windows_wasm_cache(paths)).lower()}")
