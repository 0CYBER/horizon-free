#!/usr/bin/env python3
"""The self-check to run before asking for review: one command, one block to paste into the pull request.

    python3 tools/pr_ready.py                       # on this machine, against origin/main
    python3 tools/pr_ready.py --base upstream/main  # a fork: against the main repository's main
    python3 tools/pr_ready.py --host macmini        # builds and harnesses on a fleet machine (~/fleet/rbuild)
    python3 tools/pr_ready.py --harnesses           # also run the affected vanilla harnesses here
    python3 tools/pr_ready.py --quick               # never run the vanilla harnesses

From the files the branch changes since its merge base with --base (committed or not) it decides what to run:
rustfmt, and clippy with warnings denied exactly as CI's verify job runs it (the whole workspace with the game's code
mods, plus the server build when horizon-client is affected); the tests of the touched crates, with the features
CI's workspace-wide run gives them; the launcher (its own workspace) the same way when CI's launcher job would run
(tools/ci/rust_changes.py --launcher); crates/horizon-theme/tokens.css against horizon-theme; Cargo.lock changing
only with a Cargo.toml; the Python tests (always, as in CI) and the public-tree check; and the vanilla harnesses the
changes can affect, picked as CI's parity job picks them (tools/parity.py --changed). As in CI (parity-scope in
.github/workflows/ci.yml), the harnesses run only when the branch changes a harness, an oracle, a probe or the
runner; otherwise they are listed and left to parity-main, which runs them on main. --harnesses runs them anyway.
Harnesses run where the vanilla data is (a prepared .local/ here, or the Mac mini); elsewhere they are listed for CI.
Tests select library, binary and integration targets explicitly, followed by doc tests, so unrelated example
executables are not built and linked again.

It prints one Markdown block for the pull request's body (stdout) and exits non-zero when a check fails.
Progress, and the end of a failed check's output, go to stderr (--verbose: all of it).
"""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import socket
import stat
import subprocess
import sys
import threading
import time

if os.name == "nt":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
FLEET_SHARED = Path.home() / "code" / "horizon-parity" / "shared-local" / "parity-shared"
if "PARITY_SHARED" not in os.environ and not (ROOT / ".local" / "minecraft").exists() and FLEET_SHARED.is_dir():
    # A fleet worktree without vanilla data still knows each harness's dependencies.
    os.environ["PARITY_SHARED"] = str(FLEET_SHARED)
from tools import parity, process_tree, release_train  # noqa: E402
from tools.ci import rust_changes  # noqa: E402

RBUILD = Path.home() / "fleet" / "rbuild"
# What rbuild's login shell needs on each fleet machine before cargo (it adds sccache itself). The ThinkPad no
# longer builds (2026-10-26: it keeps the AI pool and a few services; Linux work goes to playhorizon's runner).
HOST_SETUP = {
    "macmini": "source ~/.cargo/env; export PATH=/opt/homebrew/bin:$PATH CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-10}",
    "local": "",
}
# Hosts that hold the vanilla data, and how a run there gets it: CI's own runner scripts on the parity runner's
# workspace (as its second runner does), so the vanilla data, preparations, recorded harness dependencies, passes and
# machine slots are CI's; the parity state and probe builds stay per checkout.
HARNESS_SETUP = {
    "macmini": """export RUNNER_WORKSPACE="$HOME/.actions-runners/horizon/_work/horizon" RUNNER_TEMP="$(mktemp -d)"
export GITHUB_PATH="$RUNNER_TEMP/path" GITHUB_ENV="$RUNNER_TEMP/env"
bash tools/ci/runner-env.sh || exit 1
while read -r entry; do PATH="$entry:$PATH"; done < "$GITHUB_PATH"; set -a; . "$GITHUB_ENV"; set +a
bash tools/ci/vanilla-data.sh || exit 1
state="$HOME/.cache/pr-ready/$(basename "$PWD")"
mkdir -p "$state/parity-runner" "$state/parity-target"
ln -sfn "$state/parity-runner" .local/parity-runner && ln -sfn "$state/parity-target" .local/parity-target""",
}
ROOT_RUST = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}
LAUNCHER_FEATURES = """export features=""
if [ "$(uname)" = Darwin ] && ! xcrun metal --version > /dev/null 2>&1; then features="--features gpui_platform/runtime_shaders"; fi
in_shell() { if [ "$(uname)" = Linux ] && ! pkg-config --exists xkbcommon 2>/dev/null && command -v nix-shell > /dev/null; then
  nix-shell shell.nix --run "$1"; else bash -c "$1"; fi; }"""


# Build-script inputs outside the crates: catalog-only edits still need to build and test their consumers.
MOD_API_INPUTS = ("crates/horizon-mod-vocab/src/hooks/",)
# The generated dispatch surface: edits here change what the dispatch tests compile and run. The registry is its
# own crate with its own suite; a `--manifest-path` run keeps both out of the workspace selection above.
DISPATCH_TESTS = "crates/horizon-mod-api-gen/dispatch-tests/Cargo.toml"
DISPATCH_TEST_INPUTS = ("crates/horizon-mod-api-gen/", "crates/horizon-mod-api-macros/",
                        "crates/horizon-mod-api/",
                        "crates/horizon-mod-vocab/", *MOD_API_INPUTS)
MOD_REGISTRY = "crates/horizon-mod-registry/Cargo.toml"

def is_doc(path):
    return path.endswith(".md")


def workspace_rust(path):
    """Whether a change to path is built by the main Cargo workspace."""
    return not is_doc(path) and (path.startswith(("crates/", "xtask/", ".cargo/", *MOD_API_INPUTS))
                                 or path in ROOT_RUST)


def launcher_rust(path):
    """Whether a change to path is built into the launcher (its own workspace): CI's launcher job's own rule."""
    return rust_changes.needs_launcher([path])


def touched_crates(paths, packages):
    """Workspace packages to test for these paths ({name: directory}), or None for the whole workspace."""
    if any(path in ROOT_RUST or path.startswith(".cargo/") for path in paths):
        return None
    crates = set()
    for path in paths:
        if is_doc(path):
            continue
        if path.startswith(MOD_API_INPUTS):
            crates.update({"horizon-client", "horizon-client-render", "horizon-mod-api-gen"})
            continue
        if "horizon-client-lib" in packages and path.startswith("crates/horizon-client/"):
            relative = path.removeprefix("crates/horizon-client/")
            gameplay = relative in {"Cargo.toml", "build.rs", "windows-segment-heap.manifest",
                                    "src/lib.rs", "src/main.rs", "src/entry.rs", "src/game.rs", "tests/launch_api.rs"} \
                or relative.startswith("src/game/")
            crates.add("horizon-client" if gameplay else "horizon-client-lib")
            continue
        owners = [name for name, directory in packages.items() if path.startswith(directory.rstrip("/") + "/")]
        if owners:
            crates.add(max(owners, key=lambda name: len(packages[name])))
    return sorted(crates)


def area(path):
    """The part of the project a path belongs to; None for what goes with any change (docs, tests, harnesses)."""
    if is_doc(path) or path.startswith(("tests/", "mods/", "docs/", "roadmap/")) or path == "research-lock.json" or \
            (path.startswith("tools/") and not path.startswith("tools/ci/")):
        return None
    if path.startswith(("launcher/", "crates/horizon-prepare/")):
        return "launcher"
    if path.startswith((".github/", "tools/ci/")):
        return "CI"
    if path.startswith("crates/horizon-theme/"):
        return None  # the design system feeds the game, the launcher and the website (its tokens.css) alike
    return "game"


def scope_warnings(paths):
    """A pull request keeps to one area; its size is its own concern (#385)."""
    warnings = []
    areas = sorted({area(path) for path in paths} - {None})
    if len(areas) > 1:
        warnings.append("mixes " + ", ".join(areas))
    return warnings


def lock_problem(paths):
    """Cargo.lock should change only with a manifest: otherwise it is an accidental `cargo update`."""
    manifests = any(path.endswith("Cargo.toml") for path in paths)
    locks = sorted(path for path in paths if path.endswith("Cargo.lock"))
    return f"{', '.join(locks)} changed without a Cargo.toml" if locks and not manifests else None


def suggested_label(paths, exporter_paths=frozenset()):
    if any(release_train.player_facing(path, kind, exporter_paths) for path in paths for kind in ("game", "launcher")):
        return "player-facing: `feature` if players get something new, no label for a fix (`release-now` if urgent)"
    return "`skip-release`"


# CI's parity-scope (.github/workflows/ci.yml): a pull request runs its harnesses itself only when it changes a
# harness, an oracle, a probe or the runner. parity-main runs the others on main, a batch of merges at a time.
PARITY_SCOPE = re.compile(
    r"^(tools/test_[a-z0-9_]+_parity\.py|tools/(parity[a-z_]*|lab)\.py|tools/parity_hosts\.json|mods/horizon/src/test/"
    r"|crates/[^/]+/examples/|crates/horizon-client/src/game/[a-z_]*(fixture|harness)\.rs"
    r"|\.github/workflows/parity[a-z-]*\.yml)")


def runs_parity(changed):
    """Whether CI's parity job would run for a pull request that changes these files."""
    return any(PARITY_SCOPE.match(path) for path in changed)


def harness_names(names):
    """Harness names without shard suffixes, in order."""
    return list(dict.fromkeys(parity.base_name(name) for name in names))


def affected_here(changed):
    """The harnesses the changed files can affect, from the dependencies recorded here, or None if nothing is."""
    found = parity.harnesses()
    records = parity.load_state()["harnesses"]
    known = any(records.get(name, {}).get("dependencies") is not None or
                (parity.SHARED / "dependencies" / f"{name}.json").exists() for name in found)
    if not known and not changed & parity.SHARED_INPUTS:
        return None
    affected = parity.affected_harnesses(found, records, changed)
    return harness_names(name for name in found if name in affected)


def short_list(names, limit=12):
    shown = ", ".join(names[:limit])
    return shown + (f" and {len(names) - limit} more" if len(names) > limit else "")


def duration(seconds):
    seconds = int(round(seconds))
    return f"{seconds // 60} min {seconds % 60} s" if seconds >= 60 else f"{seconds} s"


def git(*arguments):
    return subprocess.run(["git", *arguments], cwd=ROOT, capture_output=True, check=True).stdout.decode("utf-8")


def shallow_clone():
    """A checkout without full history (outside a repository, `git` fails and this says no)."""
    return subprocess.run(["git", "rev-parse", "--is-shallow-repository"], cwd=ROOT, capture_output=True,
                          text=True).stdout.strip() == "true"


def line_counts(base, changed):
    """Lines added and removed in each changed path since base, untracked files included: {path: (added, removed)}."""
    counts = {}
    for row in filter(None, git("diff", "--numstat", "--no-renames", "-z", base).split("\0")):
        plus, minus, path = row.split("\t", 2)
        if plus != "-":
            counts[path] = (int(plus), int(minus))
    for path in git("ls-files", "--others", "--exclude-standard", "-z").split("\0"):
        if path in changed:
            try:
                counts[path] = (len((ROOT / path).read_text(encoding="utf-8").splitlines()), 0)
            except (UnicodeDecodeError, OSError):
                pass
    return counts


def packages():
    """Workspace directories, libraries and path dependencies, including optional/build/dev dependencies."""
    try:
        output = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT,
                                capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"pr_ready: cargo metadata failed: {getattr(error, 'stderr', error)}")
    metadata = json.loads(output)
    members = set(metadata["workspace_members"])
    workspace = [package for package in metadata["packages"] if package["id"] in members]
    directories = {package["name"]: Path(package["manifest_path"]).parent.relative_to(ROOT).as_posix()
                   for package in workspace}
    libraries = {package["name"] for package in workspace
                 if any("lib" in target["kind"] for target in package["targets"])}
    names_by_path = {Path(package["manifest_path"]).parent.resolve(): package["name"] for package in workspace}
    dependencies = {package["name"]: {names_by_path[path] for dependency in package["dependencies"]
                                     if dependency.get("path") and
                                     (path := Path(dependency["path"]).resolve()) in names_by_path}
                    for package in workspace}
    return directories, libraries, dependencies


def dependent_crates(crates, dependencies):
    """Include every workspace consumer, even through renamed or feature-gated dependencies."""
    if crates is None:
        return None
    selected = set(crates)
    while True:
        consumers = {name for name, used in dependencies.items() if selected.intersection(used)}
        if consumers <= selected:
            return sorted(selected)
        selected.update(consumers)


def prepare_key():
    """tools/ci/vanilla-data.sh's preparation key, for a copy without Git history."""
    listing = git("ls-files", "-s", "crates/horizon-prepare", "mods/horizon/src/test/java", "research-lock.json",
                  "tools/ci/vanilla-data.sh", "tools/ci/seed_prepared_extras.py", "tools/prepare_output.py")
    return hashlib.sha256(listing.encode()).hexdigest()[:16]


class Step:
    def __init__(self, name, command=None, where="host", skip=None, summary=None):
        self.name, self.command, self.where, self.skip, self.summary = name, command, where, skip, summary
        self.status, self.detail, self.seconds, self.output = ("skip", skip, None, "") if skip else (None, "", None, "")


def tests_passed(output):
    results = re.findall(r"test result: \w+\. (\d+) passed; (\d+) failed", output)
    passed, failed = (sum(int(result[i]) for result in results) for i in (0, 1))
    return f"{passed} passed" + (f", {failed} failed" if failed else "") if results else ""


def unittest_passed(output):
    failed = re.search(r"^(FAILED \(.*\))$", output, re.M)
    ran = re.search(r"^Ran (\d+) tests?", output, re.M)
    return f"{ran.group(1) if ran else '?'} run, {failed.group(1)}" if failed else f"{ran.group(1)} passed" if ran else ""


def parity_summary(output):
    """tools/parity.py's own account: how many harnesses the change affects, ran and passed, and which failed."""
    affected = re.search(r"(\d+) harnesses depend on them", output)
    if affected and affected.group(1) == "0":
        return "no harness affected"
    text = f"{affected.group(1)} affected, " if affected else ""
    if "Nothing to run" in output:
        return text + "all passed already with these inputs"
    match = re.search(r"(\d+) passed, (\d+) failed in", output)
    failed = re.findall(r"^  (\S+): \S+\.log$", output, re.M)
    skipped = re.findall(r"^skipped on \w+: ([^:]+):", output, re.M)
    text += f"{match.group(1)} passed" if match else ""
    if failed:
        text += f", failed: {', '.join(harness_names(failed))}"
    if skipped:
        text += f"; skipped on this platform: {', '.join(harness_names(skipped))}"
    return text


# CI's lint job (.github/workflows/dyna-ci.yml): collect warnings across the whole graph before rejecting them.
CI_CLIPPY = "python3 tools/ci/check_warnings.py cargo clippy --keep-going --workspace --all-targets --locked"
CI_CLIPPY_SERVER = "python3 tools/ci/check_warnings.py cargo clippy --keep-going -p horizon-client -p horizon-game -p horizon-mod-host --all-targets --locked --no-default-features"


# CI installs the current stable each run (rustup toolchain install stable), and Rust ships one every six weeks
# since 1.85.0 on 2025-02-20: so CI's version follows from the date, without the network.
RUST_TRAIN = (85, datetime.date(2025, 2, 20))


def ci_rust(today):
    """The minor version of the stable Rust CI runs on this date."""
    minor, released = RUST_TRAIN
    return minor + (today - released).days // 42


def rust_version():
    """The rustc the checks here run with, as (major, minor, patch), or None."""
    try:
        output = subprocess.run(["rustc", "--version"], cwd=ROOT, capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    match = re.match(r"rustc (\d+)\.(\d+)\.(\d+)", output)
    return tuple(int(part) for part in match.groups()) if match else None


def old_rust(version, today):
    """A warning when this Rust is older than CI's: an older clippy flags lints CI's no longer does."""
    if version is None or version[:1] != (1,) or version[1] >= ci_rust(today):
        return None
    return (f"{'.'.join(map(str, version))} is older than CI's stable (1.{ci_rust(today)}): clippy may flag lints "
            "CI doesn't, update with `rustup update`")


def mark_old_rust(steps, warning):
    """A ⚠️ Rust row above the first clippy that runs, and failed clippy rows point at it. Never fails the run."""
    clippy = [step for step in steps if step.name.endswith("clippy") and step.skip is None]
    if not warning or not clippy:
        return
    for step in clippy:
        if step.status == "fail":
            step.detail += "; likely the old Rust above"
    rust = Step("Rust", where="git")
    rust.status, rust.detail = "warn", warning
    steps.insert(steps.index(clippy[0]), rust)


def mod_features(crates):
    """The features CI's workspace-wide test run gives these crates. Cargo unifies features over the packages it
    builds, so a crate tested alone needs the ones the workspace would have turned on for it: horizon-mod-package' index."""
    return " --features horizon-mod-package/index" if crates is not None and "horizon-mod-package" in crates else ""


# What horizon-client's full content-root tests load (mod_content_tests.rs). They take minutes on prepared data, so
# they are #[ignore]d and run here only when one of these changes.
CONTENT_ROOT = ("crates/horizon-client/src/mod_content", "crates/horizon-client-render/src/assets.rs", "crates/horizon-items/",
                "crates/horizon-mod-package/", "crates/horizon-assets/")
CONTENT_ROOT_TESTS = "cargo test --locked -p horizon-client-lib --lib mod_content::tests:: -- --ignored"

# holder::tests spawns target/<profile>/examples/holder_actor beside its test binary.
# --tests does not build that executable; use the same explicit prerequisite as CI.
HOLDER_ACTOR = "cargo build --locked -p horizon-mod-package --features runtime-process --example holder_actor"


def plan(args, base, changed, crates_of, libraries, dependencies):
    """The checks for these changed files, in the order they run."""
    rust = any(workspace_rust(path) for path in changed)
    launcher = any(launcher_rust(path) for path in changed)
    # The Python tests also hold ROADMAP.md to roadmap/roadmap.toml (tests/test_roadmap.py).
    crates = touched_crates(changed, crates_of) if rust else []
    # An unrecognised Rust path must not silently skip validation.
    if rust and crates == []:
        crates = None
    selection = "--workspace" if crates is None else " ".join(f"-p {shlex.quote(c)}" for c in crates)
    # CI's own clippy commands: a crate checked alone gets other features than in CI's workspace-wide run, and an
    # unused import behind a feature passed here and failed in CI (#1895). Warm, the workspace run costs about what
    # the touched crates and their dependents did (14-32 s against 14-17 s on the Mac mini, 2026-10-26).
    clippy_crates = dependent_crates(crates, dependencies)
    clippy = CI_CLIPPY
    if clippy_crates is None or "horizon-client" in clippy_crates:
        clippy = f'status=0\n{CI_CLIPPY} || status=$?\n{CI_CLIPPY_SERVER} || status=$?\nexit "$status"'
    test_skip = None if crates is None or crates else "no crate touched"
    doc_crates = sorted(libraries if crates is None else libraries.intersection(crates))
    doc_selection = "--workspace" if crates is None else " ".join(f"-p {shlex.quote(c)}" for c in doc_crates)
    no_rust = "no Rust change"
    steps = [
        Step("Cargo.lock", where="git"),
        Step("Public tree", "python3 tools/check_public_tree.py", where="here"),
        Step("Repository hygiene", "python3 tools/ci/check_repository_hygiene.py", where="here"),
        # docs/layers.md: a crate depends only on its own layer or a lower one, and the exceptions only shrink.
        Step("Layers", f"python3 tools/check_layers.py --base {shlex.quote(base)}", where="here"),
        Step("Addresses", f"python3 tools/check_addresses.py --base {shlex.quote(base)}", where="here"),
        # The provenance check needs full history and fails closed on a shallow clone; CI checks a full one.
        Step("File size", f"python3 tools/check_file_size.py --base {shlex.quote(base)}", where="here",
             skip="not checked on a shallow clone" if shallow_clone() else None),
        # tools/hotspots/README.md: export brace lists stay at 12 names or under; longer ones only shrink.
        Step("Export lists", "python3 tools/hotspots/check_lists.py", where="here"),
        # tools/hotspots/check_stand_ins.py: test stand-ins keep up with the hooks and traits they mirror.
        Step("Test stand-ins", "python3 tools/hotspots/check_stand_ins.py", where="here"),
        # Always, as in CI: some read the crates, the roadmap or the workflows.
        Step("Python tests", "python3 -m unittest discover -s tests", where="here", summary=unittest_passed),
        # CI's own scan (tools/ci/format_scan.py): cargo fmt, then the files no `mod` line reaches, by path.
        Step("rustfmt", "python3 tools/ci/format_scan.py", skip=None if rust else no_rust),
        Step("clippy", clippy, skip=None if rust else no_rust),
        Step("holder actor", HOLDER_ACTOR,
             skip=None if crates is None or "horizon-mod-package" in crates or launcher
             else "holder tests untouched"),
        Step("tests" + (": workspace" if crates is None else f": {', '.join(crates)}" if crates else ""),
             f"cargo test --locked {selection}{mod_features(crates)} --tests",
             skip=test_skip, summary=tests_passed),
        Step("doc tests", f"cargo test --locked {doc_selection} --doc",
             skip=test_skip or (None if doc_crates else "no library touched"), summary=tests_passed),
        Step("content root tests", CONTENT_ROOT_TESTS, summary=tests_passed,
             skip=None if crates is None or any(p.startswith(CONTENT_ROOT) for p in changed)
             else "content root untouched"),
        Step("tokens.css up to date", "cargo test --locked -p horizon-theme --lib tokens_css_is_up_to_date",
             skip=None if any(p.startswith(("crates/horizon-theme/", "xtask/")) for p in changed)
             else "theme untouched"),
    ]
    dispatch_skip = None if any(path.startswith(DISPATCH_TEST_INPUTS) for path in changed) \
        else "dispatch surface untouched"
    registry_skip = None if any(path.startswith("crates/horizon-mod-registry/") for path in changed) \
        else "mod registry untouched"
    for name, manifest, skip in (("dispatch tests", DISPATCH_TESTS, dispatch_skip),
                                 ("mod registry tests", MOD_REGISTRY, registry_skip)):
        steps.extend([
            Step(f"{name} clippy",
                 f"CARGO_BUILD_JOBS=${{CARGO_BUILD_JOBS:-4}} python3 tools/ci/check_warnings.py cargo clippy --keep-going --manifest-path {manifest}"
                 " --all-targets --locked", skip=skip),
            Step(f"{name} tests",
                 f"CARGO_BUILD_JOBS=${{CARGO_BUILD_JOBS:-4}} cargo test --manifest-path {manifest}"
                 " --locked -- --test-threads=4", skip=skip, summary=tests_passed),
        ])
    launcher_skip = None if launcher else "launcher untouched"
    for name, command in [("launcher rustfmt", "cargo fmt --check"),
                          ("launcher clippy", "python3 ../tools/ci/check_warnings.py cargo clippy --keep-going --all-targets --locked $features"),
                          ("launcher tests", "cargo test --locked --lib --bins $features")]:
        steps.append(Step(name, f"cd launcher && {LAUNCHER_FEATURES}\nin_shell {shlex.quote(command)}", skip=launcher_skip,
                          summary=tests_passed if name == "launcher tests" else None))
    harness = Step("Parity", summary=parity_summary)
    affected = affected_here(changed)
    local_data = (ROOT / ".local" / "minecraft").exists() and (ROOT / ".local" / "horizon-client").exists()
    in_scope = args.harnesses or runs_parity(changed)
    if not args.quick and not in_scope:
        # What CI does with this pull request: nothing here, parity-main on main.
        harness.status = "list"
        why = "no harness, oracle, probe or runner change; --harnesses runs them here"
        if affected is None:
            harness.detail = f"not run ({why}): parity-main checks this on main"
        elif not affected:
            harness.detail = "no harness affected"
        else:
            harness.detail = f"{len(affected)} affected, left to parity-main on main ({why}): {short_list(affected)}"
    elif args.quick or (args.host == "local" and not local_data) or (args.host != "local" and args.host not in HARNESS_SETUP):
        harness.status = "list"
        why = "--quick" if args.quick else "no vanilla data here"
        if affected is None:
            harness.detail = f"not run ({why}): CI's parity job picks the affected harnesses"
        elif not affected:
            harness.detail = "no harness affected"
        else:
            harness.detail = f"{len(affected)} affected, to run in CI ({why}): {short_list(affected)}"
    elif affected == []:
        harness.status, harness.detail = "pass", "no harness affected"
    elif args.host == "local":
        harness.where, harness.command = "here", f"PARITY_HOSTS= python3 tools/parity.py --changed {shlex.quote(base)} --timeout 1500"
    else:
        listing = "\n".join(sorted(changed))
        harness.command = f"""export HORIZON_PREPARE_KEY={prepare_key()}
{HARNESS_SETUP[args.host]}
cat > "$RUNNER_TEMP/changed" <<'PR_READY_CHANGED'
{listing}
PR_READY_CHANGED
PARITY_HOSTS= python3 tools/parity.py --changed-files "$RUNNER_TEMP/changed" --timeout 1500"""
    datafix = getattr(args, 'datafix', False)
    harness.datafix_selected = datafix
    if not datafix:
        print('datafix parity not run: opt in with --datafix and a private HORIZON_DATAFIX_ROOT (default checkout .local/datafix).', file=sys.stderr)
        if harness.command:
            harness.command += ' --exclude datafix'
    steps.append(harness)
    return steps


CLIPPY_CACHE_HINT = (
    "pr_ready: the first clippy run builds the whole workspace once (cold, can take 20+ min); later runs are warm. "
    "Set RUSTC_WRAPPER=sccache with CARGO_INCREMENTAL=0 to reuse dependency builds across branches and checkouts."
)


def run_script(steps, host, verbose):
    """Runs the steps in one shell on host (here for "local"), each between markers, reading results as they come."""
    if not steps:
        return
    # Explicit local opt-in can select datafix even with unknown dependencies.
    # Preserve the validated absolute path across the subprocess cwd change.
    environment = os.environ.copy()
    if host == 'local' and any(step.name == 'Parity' and getattr(step, 'datafix_selected', False) for step in steps):
        from tools.datafix_paths import VARIABLE, writable_root
        environment[VARIABLE] = str(writable_root())
    if host == "local" and os.name == "nt":
        run_windows(steps, verbose, environment)
        return
    parts = [HOST_SETUP.get(host, ""), "set +e"]
    first_clippy = True
    for index, step in enumerate(steps):
        if step.name.endswith("clippy") and first_clippy:
            first_clippy = False
            parts.append('case "${RUSTC_WRAPPER:-}" in sccache|*/sccache|sccache.exe|*/sccache.exe) ;; '
                         f'*) printf \'%s\\n\' {shlex.quote("@@pr_ready:hint:" + CLIPPY_CACHE_HINT)} ;; esac')
        parts.append(f"echo '@@pr_ready:begin:{index}'; started=$(date +%s)\n( {step.command}\n) 2>&1 < /dev/null\n"
                     f"status=$?; echo; echo \"@@pr_ready:end:{index}:$status:$(( $(date +%s) - started ))\"")
    script = "\n".join(parts)
    if host == "local":
        command = ["bash", "-c", script]
    else:
        command = [str(RBUILD), host, "--", "bash", "-c", script]
    current = None
    with subprocess.Popen(command, cwd=ROOT, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                          errors="replace") as process:
        for line in process.stdout:
            marker = re.match(r"@@pr_ready:(begin|end):(\d+)(?::(\d+):(\d+))?$", line.strip())
            if marker and marker.group(1) == "begin":
                current = steps[int(marker.group(2))]
                print(f"  {current.name}...", file=sys.stderr, flush=True)
            elif marker:
                step = steps[int(marker.group(2))]
                step.status = "pass" if marker.group(3) == "0" else "fail"
                step.seconds = int(marker.group(4))
                finish(step, verbose)
                current = None
            elif line.startswith("@@pr_ready:hint:"):
                print(line.removeprefix("@@pr_ready:hint:"), end="", file=sys.stderr, flush=True)
            elif current is not None:
                current.output += line
                if verbose:
                    print("    " + line, end="", file=sys.stderr)
            elif verbose or host != "local":
                print("    " + line, end="", file=sys.stderr)
    for step in steps:
        if step.status is None:
            step.status, step.detail = "fail", ("did not finish" if step.output else f"not run: {host} unreachable?")
            finish(step, verbose)


def run_windows(steps, verbose, environment=None):
    """Run local checks directly on Windows without requiring a Unix shell. Each runs in its own job
    (tools/process_tree.py): what a check starts, a harness's game say, ends with it, or with this process."""
    first_clippy = True
    for step in steps:
        command = shlex.split(step.command)
        cwd = ROOT
        env = (os.environ if environment is None else environment).copy()
        env["PYTHONUTF8"] = "1"
        if command and command[0].startswith("CARGO_BUILD_JOBS="):  # the shell form, with no shell to read it
            command.pop(0)
            env.setdefault("CARGO_BUILD_JOBS", "4")
        if step.name == "Public tree":
            command = [sys.executable, "tools/check_public_tree.py"]
        elif step.name == "Layers":
            command = [sys.executable, "tools/check_layers.py", *command[2:]]
        elif step.name == "Addresses":
            command = [sys.executable, "tools/check_addresses.py", *command[2:]]
        elif step.name == "File size":
            command = [sys.executable, "tools/check_file_size.py", *command[2:]]
        elif step.name == "Export lists":
            command = [sys.executable, "tools/hotspots/check_lists.py"]
        elif step.name == "Test stand-ins":
            command = [sys.executable, "tools/hotspots/check_stand_ins.py"]
        elif step.name == "Python tests":
            command = [sys.executable, "-m", "unittest", "discover", "-s", "tests"]
        elif step.name == "rustfmt":
            command = [sys.executable, "tools/ci/format_scan.py"]
        elif step.name == "clippy":
            command = [sys.executable, *shlex.split(CI_CLIPPY)[1:]]  # server clippy runs in CI (Linux)
        elif step.name.startswith("tests"):
            command = shlex.split(step.command)
        elif step.name == "tokens.css up to date":
            command = shlex.split(step.command)
        elif step.name == "launcher rustfmt":
            command = ["cargo", "fmt", "--check"]
            cwd = ROOT / "launcher"
        elif step.name == "launcher clippy":
            command = [sys.executable, "../tools/ci/check_warnings.py", "cargo", "clippy", "--keep-going",
                       "--all-targets", "--locked"]
            cwd = ROOT / "launcher"
        elif step.name == "launcher tests":
            command = ["cargo", "test", "--locked", "--lib", "--bins"]
            cwd = ROOT / "launcher"
        elif step.name == "Parity":
            command = shlex.split(step.command)
            if command and command[0] == "PARITY_HOSTS=":
                command.pop(0)
            if command and command[0] == "python3":
                command[0] = sys.executable
            env["PARITY_HOSTS"] = ""
        elif command and command[0] == "python3":
            command[0] = sys.executable

        if step.name.endswith("clippy") and first_clippy:
            first_clippy = False
            if Path(env.get("RUSTC_WRAPPER", "")).name not in ("sccache", "sccache.exe"):
                print(CLIPPY_CACHE_HINT, file=sys.stderr, flush=True)
        print(f"  {step.name}...", file=sys.stderr, flush=True)
        started = time.monotonic()
        try:
            with process_tree.popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, text=True, errors="replace") as process:
                def release(process):
                    # The check's exit ends what it left running, which would otherwise hold the output open.
                    process.wait()
                    process_tree.ended(process)
                # A daemon: interrupted, this process exits at once, and its jobs end the checks with it.
                reaper = threading.Thread(target=release, args=(process,), daemon=True)
                reaper.start()
                for line in process.stdout:
                    step.output += line
                    if verbose:
                        print("    " + line, end="", file=sys.stderr)
                reaper.join()
                status = process.returncode
            step.status = "pass" if status == 0 else "fail"
        except OSError as error:
            step.status, step.detail = "fail", str(error)
        step.seconds = int(time.monotonic() - started)
        finish(step, verbose)


def run_checks(steps, host, verbose):
    """Runs the checks: the Python ones here beside the cargo ones on host, then the harnesses.

    The Python tests and the public-tree check only read files, and they took about two minutes of a warm run that
    waited for them before cargo started (114 of 213 s, 2026-11-03). Side by side, the run takes about as long as the
    longer of the two. The harnesses build with cargo, so they still come after it. With --verbose, every check's
    output is shown, and the checks run one after another so that it doesn't interleave.
    """
    here = [step for step in steps if step.where == "here" and step.status is None]
    beside = [step for step in here if step.name != "Parity"]
    after = [step for step in here if step.name == "Parity"]
    on_host = [step for step in steps if step.where == "host" and step.status is None]
    if verbose:
        run_script(beside + after, "local", verbose)
        run_script(on_host, host, verbose)
        return
    python = threading.Thread(target=run_script, args=(beside, "local", verbose))
    python.start()
    try:
        run_script(on_host, host, verbose)
    finally:
        python.join()
    run_script(after, "local", verbose)


def finish(step, verbose):
    if step.status == "pass" and step.summary:
        step.detail = step.summary(step.output)
    elif step.status == "fail":
        step.detail = (step.summary(step.output) if step.summary else "") or step.detail or "failed"
        if not verbose and step.output:
            print("".join(step.output.splitlines(keepends=True)[-40:]), end="", file=sys.stderr)
    took = f" ({duration(step.seconds)})" if step.seconds is not None else ""
    print(f"  {step.name}: {step.status}{took}", file=sys.stderr, flush=True)


MARK = {"pass": "✅", "fail": "❌", "skip": "➖", "list": "➖", "warn": "⚠️"}


def revision_state():
    """Snapshot before selection and after checks; ignored build output is excluded."""
    untracked = git("ls-files", "--others", "--exclude-standard", "-z").split("\0")[:-1]
    metadata = []
    for path in untracked:
        # Never open untracked content: symlinks can dangle or name directories,
        # and FIFOs/devices must not be read. Dirty starts are uncertified anyway.
        try:
            info = (ROOT / path).lstat()
            link = os.readlink(ROOT / path) if stat.S_ISLNK(info.st_mode) else None
            metadata.append((path, info.st_mode, info.st_size, info.st_mtime_ns, info.st_ctime_ns, link))
        except OSError:
            metadata.append((path, "unavailable"))
    return (git("rev-parse", "HEAD").strip(),
            git("status", "--porcelain=v1", "--untracked-files=all"),
            git("diff", "HEAD", "--binary") + repr(metadata))


def block(args, base, merge_base, changed, counts, steps, label, elapsed, revision, changed_during_run=False):
    where = socket.gethostname().split(".")[0] if args.host == "local" else args.host
    head, status, _ = revision
    dirty = bool(status)
    flags = "".join(f" --{flag}" for flag in ("quick", "harnesses") if getattr(args, flag)) + \
        ("" if args.host == "local" else f" --host {args.host}") + ("" if base == "origin/main" else f" --base {base}")
    added, removed = (sum(count[i] for count in counts.values()) for i in (0, 1))
    warnings = scope_warnings(changed)
    lines = [f"**pr_ready** (`tools/pr_ready.py{flags}`, commit `{head}`{' dirty' if dirty else ''}, "
             f"{'changed during run; checks do not certify the ending worktree, ' if changed_during_run else ''}"
             f"host `{where}`, against {base} at {merge_base[:9]}, "
             f"{duration(elapsed)})", "", "| Check | Result |", "| --- | --- |",
             f"| Scope | {'⚠️' if warnings else '✅'} {len(changed)} files, +{added} −{removed}"
             + "".join(f"; {warning}" for warning in warnings) + " |"]
    for step in steps:
        took = f" ({duration(step.seconds)})" if step.seconds else ""
        detail = f" {step.detail}" if step.detail else ""
        lines.append(f"| {step.name} | {MARK[step.status]}{detail}{took} |")
    content = next((step for step in steps if step.name == "content root tests"), None)
    resource_override = os.environ.get("HORIZON_TEST_RESOURCES")
    resource_roots = [Path(resource_override)] if resource_override is not None else [
        ROOT / ".local/horizon-client/26.3", ROOT.parent.parent / ".local/horizon-client/26.3"]
    if args.host == "local" and not any((root / "gameplay-facts.json").is_file() for root in resource_roots):
        data_tests = "data-backed tests not run: no prepared data here"
    elif content and content.status in {"pass", "fail"}:
        data_tests = f"data-backed tests: horizon-client-lib mod_content::tests:: (--ignored): {content.detail or content.status}"
    else:
        data_tests = "data-backed tests not run: content root untouched"
    lines.append(f"| Data-backed cargo tests | {data_tests} |")
    lines.append(f"| Label | {label} |")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", default="origin/main", help="what the pull request merges into (default origin/main)")
    parser.add_argument("--host", default="local", choices=sorted(HOST_SETUP),
                        help="where builds, tests and harnesses run: local, or a fleet machine through ~/fleet/rbuild")
    parser.add_argument("--quick", action="store_true", help="list the affected harnesses instead of running them")
    parser.add_argument("--harnesses", action="store_true",
                        help="run the affected harnesses even when the branch changes no harness, oracle, probe or runner")
    parser.add_argument("--verbose", action="store_true", help="show every check's output")
    parser.add_argument('--datafix', action='store_true', help='opt in to datafix parity downloads in a private HORIZON_DATAFIX_ROOT')
    args = parser.parse_args()
    if args.host != "local" and not RBUILD.exists():
        parser.error(f"--host {args.host} needs {RBUILD}; without the fleet, run it here (no --host)")
    started = time.monotonic()
    revision = revision_state()
    changed = parity.changed_files(args.base)
    if changed is None:
        parser.error(f"no merge base with {args.base} (git fetch?)")
    merge_base = git("merge-base", "HEAD", args.base).strip()
    counts = line_counts(merge_base, changed)
    crates_of, libraries, dependencies = packages() if any(workspace_rust(p) for p in changed) else ({}, set(), {})
    steps = plan(args, args.base, changed, crates_of, libraries, dependencies)
    print(f"pr_ready: {len(changed)} files changed since {args.base}; checks on {args.host}", file=sys.stderr, flush=True)

    lock = steps[0]
    problem = lock_problem(changed)
    lock.status, lock.detail = ("fail", problem) if problem else ("pass", "unchanged" if not any(
        p.endswith("Cargo.lock") for p in changed) else "changes with a Cargo.toml")
    run_checks(steps, args.host, args.verbose)
    warning = old_rust(rust_version(), datetime.date.today()) if args.host == "local" else None
    if warning and any(step.name.endswith("clippy") and step.skip is None for step in steps):
        print(f"pr_ready: Rust {warning}", file=sys.stderr)
    mark_old_rust(steps, warning)

    label = suggested_label(changed, release_train.exporters())
    print(block(args, args.base, merge_base, sorted(changed), counts, steps, label,
                time.monotonic() - started, revision, revision_state() != revision))
    return 1 if any(step.status == "fail" for step in steps) else 0


if __name__ == "__main__":
    sys.exit(main())
