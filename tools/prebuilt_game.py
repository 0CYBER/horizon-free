#!/usr/bin/env python3
"""The prebuilt game (arismoko/horizon#589): horizon-client as a Rust dylib plus every crate's metadata, so the launcher builds
native trait mods against it with rustc alone, in seconds, without the game's source or a game build.

    python3 tools/prebuilt_game.py build OUT [--server]   # build and collect the folder, write horizon-prebuilt.json
    python3 tools/prebuilt_game.py smoke OUT              # build the starter mod and the game executable against OUT only
    python3 tools/prebuilt_game.py start OUT              # start smoke's executable, as installed, as far as no game data allows
    python3 tools/prebuilt_game.py exports OUT            # each DLL's exported names, toward Windows' limit
    python3 tools/prebuilt_game.py pack OUT DIST          # the SDK's assets from OUT: runtime-, api- and
                                                          # engine-<target>[-server].tar.xz, build-data.tar.xz
                                                          # and surface.json, in DIST
    python3 tools/prebuilt_game.py gather LEGS DIST       # the release folder from every packing leg's DIST, once
                                                          # their build data and surface are checked equal
    python3 tools/prebuilt_game.py index DIST [--nightly NAME] [--previous BUILD DIFF]  # DIST/sdk.json

Each OS's folder is built on that OS: the proc-macro libraries in it run inside the player's own rustc.
The folder holds compiler outputs and game-owned registry patch crates, never vanilla data; `build` refuses any other file.
"""

import argparse
from concurrent.futures import ProcessPoolExecutor
import hashlib
import json
import lzma
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
PROFILE = "release"
# What rustc writes for a dylib or proc macro on each OS, plus metadata and the MinGW import libraries.
SUFFIXES = (".rmeta", ".so", ".dll", ".dll.a", ".dylib")
IDENTITY = "horizon-prebuilt.json"
# The crate the game's library is built from; its build resolves what mods compile against.
GAME = "horizon-mod-host"
# Horizon's official parts (docs/layers.md, "What players get"): a client's executable registers them, built with
# the game's library but linked into the executable only.
OFFICIAL = "horizon-official"
# Crates the parts and the game's library must share, never compile twice: the hooks' registries, the facade the
# parts reach them through, and the two crates both the game and a part use (Play with Friends, telemetry).
SHARED_ONCE = ("horizon_hooks", "horizon_platform", "horizon_friends", "horizon_telemetry")
# What a start without prepared files prints only when the official parts registered with the game's hooks:
# horizon-official-servers' `get_the_game` (loading_screen.rs `missing_resources`).
OFFICIAL_PARTS = "Horizon is started from its launcher"
LOCK = "Cargo.lock"
# Where `smoke` installs the executable it builds, as the launcher does, for `start`: beside OUT, never inside it,
# since the archive is OUT alone.
SMOKE = "prebuilt-smoke"
# Where `smoke` installs the same executable without selected mods, for `start`.
SMOKE_API = "prebuilt-smoke-api"
EXECUTABLE = "horizon-client.exe" if sys.platform == "win32" else "horizon-client"
# The furthest a game started without its prepared files gets (game.rs `missing_resources`): past loading the
# game's libraries and the mods' runtime environment (`entry::main_with`), where a split allocator aborts (arismoko/horizon#3877).
NO_RESOURCES = "Horizon's prepared game files are missing"
# The MinGW runtime the game's GNU DLLs import on Windows, installed beside the executable.
MINGW_RUNTIME = ("libgcc_s_seh-1.dll", "libwinpthread-1.dll")
# Windows' STATUS_DLL_NOT_FOUND, the exit status of a process whose imports could not all be loaded.
DLL_NOT_FOUND = 0xC0000135
# The launcher's generated `main.rs` comes from the game it links against.
TEMPLATE = "native_main.rs.template"
NATIVE_MODS_TEMPLATE = "native_mod_entry.rs.template"
API_MAIN = ROOT / "crates/horizon-client/src/native_main.rs.template"
# The fixed executable folder in the runtime slice.
RUNTIME = "bin"
# The required inputs (`horizon_mod_package::sdk::Inputs`): `members` lists what each slice must hold.
INPUTS = ROOT / "crates/horizon-mod-package/src/sdk/inputs.json"
# The crates an API-only mod may name; their closure is the `api` archive. `horizon-mod-points` is empty since the
# injection points were removed (#19576); it stays while launchers released before then link it into every native mod.
API_ROOTS = ("horizon-mod-api", "horizon-mod-points")
# The game libraries outside the API closure, in the runtime archive.
GAME_LIBRARIES = ("horizon_mod_host", "horizon_mod_wasm")
# Source files the folder carries besides compiler outputs.
CARRIED = {"Cargo.toml": None, IDENTITY: None, LOCK: ROOT / LOCK, TEMPLATE: ROOT / "crates/horizon-client/src" / TEMPLATE,
           NATIVE_MODS_TEMPLATE: ROOT / "crates/horizon-client/src" / NATIVE_MODS_TEMPLATE}
# What the game's build script asks the `horizon-client` executable's link for (Windows' 8 MB main stack).
LINK_ARG = "cargo:horizon-executable-link-arg="
# Native libraries the build scripts point the final link at (the `windows` crates' MinGW import libraries):
# cargo passes their folders to rustc, a plain rustc gets them from here (`-L native=OUT/native`).
NATIVE = "native"
NATIVE_SUFFIXES = (".a", ".lib")
# The game's injection points, listed by the staging pass: the launcher routes mods' point injections by it.
POINTS = "points.json"
# The staged internals signatures, checked against a mod's declared access list.
INTERNALS = "internals.json"
# The published loops (`#[horizon_loop]`), which a mod's `[[native.loop]]` rows name.
LOOPS = "loops.json"
# The function index: where each function a mixin may target is, and whether it takes a replacement; the
# launcher's patch step reads it instead of walking the game's source. Only the engine route patches functions, so it
# is in the `engine` slice, not with the rest of the build data in `api`.
FUNCTIONS = "functions.json"
# The SDK's build data (`mod_surface::build_data`): each file opens with `schema` and `build.commit`.
BUILD_DATA = (FUNCTIONS, INTERNALS, LOOPS, POINTS)
# The build data the API route reads, in the `api` slice.
API_BUILD_DATA = (INTERNALS, LOOPS, POINTS)
BUILD_DATA_SCHEMA = 1
BUILD_DATA_ARCHIVE = "build-data.tar.xz"
# The surface export (`horizon-stage surface`), an SDK asset of its own.
SURFACE = "surface.json"
# xz's default preset: the dictionary covers the large metadata files; the publish legs' time is measured by PR 5.
XZ_PRESET = 6


def run(command: list[str], **kwargs) -> str:
    print("+", " ".join(command), flush=True)
    return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE, **kwargs).stdout


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def rustc_host() -> str:
    verbose = run(["rustc", "-vV"])
    return next(line.split(": ", 1)[1] for line in verbose.splitlines() if line.startswith("host: "))


def package_format(root: Path) -> int:
    """The package layout's one number, from its one place (`sdk::PACKAGE_FORMAT` includes the same file)."""
    text = (root / "crates/horizon-mod-package/src/sdk/package-format").read_text().rstrip()
    if not re.fullmatch(r"[0-9]+", text):
        raise ValueError(f"sdk/package-format must hold one number, not {text!r}")
    return int(text)


def tracked_blobs(root: Path, commit: str, paths: tuple[str, ...] = ()):
    """Each regular file `commit` tracks under `paths` (everywhere when empty), with its bytes as the commit holds them:
    the bytes GitHub's archive of the commit serves, which the launcher downloads and checks against the function
    index, whatever line endings the working tree was checked out with (Git for Windows' core.autocrlf) and whatever
    it has that is not committed. Links are left out, as the launcher's download leaves them out."""
    # `git archive` converts line endings as a checkout would (core.autocrlf, core.eol); both are turned off so the
    # bytes are the commit's on every runner.
    with subprocess.Popen(["git", "-c", "core.autocrlf=false", "-c", "core.eol=lf", "archive", "--format=tar",
                           commit, *(["--", *paths] if paths else [])],
                          cwd=root, stdout=subprocess.PIPE) as process, process.stdout as stream, \
            tarfile.open(fileobj=stream, mode="r|") as archive:
        for member in archive:
            if member.isfile():
                yield member.name, archive.extractfile(member).read()
    if process.returncode != 0:
        sys.exit(f"git archive {commit} failed in {root}")


def head() -> str:
    return run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip()


def prebuilt_target() -> Path:
    """Cargo's target folder for the prebuilt game; the staging tool builds in its `tools` folder."""
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "prebuilt"


# `horizon-stage`, built and run by cargo in the prebuilt target's `tools` folder.
STAGE_TOOL = ["cargo", "run", "--locked", "--release", "-q", "-p", "horizon-mod-api-gen", "--bin", "horizon-stage", "--"]


def staged_tool() -> Path:
    """The `horizon-stage` that staging built ([`STAGE_TOOL`] in `prebuilt_target() / "tools"`)."""
    return prebuilt_target() / "tools" / "release" / ("horizon-stage.exe" if sys.platform == "win32" else "horizon-stage")


def surface_tool() -> tuple[list[str], dict | None]:
    """How `pack` runs horizon-stage, and with which environment (None: this one).

    The binary staging built, as it is: Cargo from the checkout would build a second copy (up to 2.6 min per SDK leg),
    since the staged copy's own .cargo/config.toml adds its rustflags to the checkout's. Without one, Cargo builds it
    in staging's target folder. `pack` checks the package is this commit's; a tool left by another commit's staging in
    the same target folder is not detected."""
    tool = staged_tool()
    if tool.is_file():
        print(f"Surface export with the staged tool {tool}", flush=True)
        return [str(tool.resolve())], None
    print("No staged tool: Cargo builds horizon-stage for the surface export", flush=True)
    return STAGE_TOOL, {**os.environ, "CARGO_TARGET_DIR": str(prebuilt_target() / "tools")}


# Set (to anything) for `cargo rustc --timings` on the SDK build: dry runs only (prebuilt-game-build.yml). Cargo fails
# the build when it can't write the report, so a release build never asks for one.
TIMINGS = "HORIZON_SDK_TIMINGS"


def stage(target_dir: Path, stage_tool: Path | None = None, prepare=None) -> Path:
    """A copy of HEAD's tracked source with the staging pass run over it (arismoko/horizon#589): every point method gets its
    `#[point]`, internals become public, and the build data is written with HEAD's commit in its header.
    The copy is the commit's own bytes ([`tracked_blobs`]), never the working tree's, which is never rewritten.

    The copy is built in `stage.next` and synced into `stage`, writing only the files whose bytes
    changed. Cargo decides freshness by modification time, so a fresh copy every run rebuilt every
    workspace crate."""
    setup_started = time.monotonic()
    final = target_dir / "stage"
    staged = target_dir / "stage.next"
    if staged.exists():
        shutil.rmtree(staged)
    commit = head()
    for name, data in tracked_blobs(ROOT, commit):
        (staged / name).parent.mkdir(parents=True, exist_ok=True)
        (staged / name).write_bytes(data)
    arguments = [str(staged), str(staged / POINTS), str(staged / INTERNALS), commit]
    command = [str(stage_tool.resolve())] if stage_tool is not None else STAGE_TOOL
    print(f"stage_phase=copy_setup seconds={time.monotonic() - setup_started:.3f}", flush=True)
    started = time.monotonic()
    print(run(command + arguments, cwd=staged,
              env=None if stage_tool is not None else
              {**os.environ, "CARGO_TARGET_DIR": str(target_dir / "tools")}), end="")
    label = "Stage binary execution" if stage_tool is not None else "Stage release build and execution"
    print(f"{label}: {time.monotonic() - started:.3f}s (source copy/setup excluded)", flush=True)
    # Edits that must land before the sync, so they are not a change on every run.
    if prepare is not None:
        prepare(staged)
    sync_unchanged(staged, final)
    shutil.rmtree(staged)
    # Only the prebuilt game splits the wasm runtime out: normal source builds stay static. With
    # prefer-dynamic, rustc picks this dylib for the game rather than folding its runtime stack into it.
    return final


def sync_unchanged(source: Path, destination: Path) -> tuple[int, int, int]:
    """Make `destination` hold exactly `source`'s files, writing only those whose bytes differ, so an
    unchanged file keeps its modification time (and Cargo its compiled crate). A top-level `target`
    directory is left alone. Returns `(written, removed, unchanged)`."""
    wanted = {Path(folder).relative_to(source) / name
              for folder, _, files in os.walk(source) for name in files}
    # Clear stale paths first: a tracked path may become a directory, or vice versa.
    removed = 0
    if destination.exists():
        folders = []
        for folder, directories, files in os.walk(destination):
            relative = Path(folder).relative_to(destination)
            if relative == Path("."):
                # Pruned before descent, so Cargo's cache is never walked.
                directories[:] = [name for name in directories if name != "target"]
            else:
                folders.append(Path(folder))
            for name in files:
                if relative / name not in wanted:
                    (Path(folder) / name).unlink()
                    removed += 1
        # Deepest first, so a folder emptied of its subfolders goes too.
        for path in reversed(folders):
            if not any(path.iterdir()):
                path.rmdir()
    written = 0
    for name in wanted:
        data = (source / name).read_bytes()
        target = destination / name
        try:
            previous = target.read_bytes()
        except FileNotFoundError:
            previous = None
        if previous != data:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            written += 1
    unchanged = len(wanted) - written
    print(f"stage_sync written={written} removed={removed} unchanged={unchanged}", flush=True)
    return written, removed, unchanged


def cargo_features(server: bool, platform: str) -> list[str]:
    """The game crate's features for this archive. The archive's std is a shared library with its own
    default-allocator `__rust_alloc`, and the wasm runtime's library, below the game, binds to it. Only ELF's
    dynamic linker (Linux) resolves every library's `__rust_alloc` to the first definition, the game's mimalloc.
    Windows (arismoko/horizon#2394) and macOS (arismoko/horizon#3877) bind each import to the library it was linked against, so a game allocator
    there leaves the wasm runtime, the mods' executable and std's own code on std's heap beside it, and a block
    freed on the other heap aborts. So everywhere but Linux the game keeps no allocator of its own and every
    library shares std's (the system heap)."""
    features = ["--no-default-features"] if server else []
    if platform != "linux":
        features += ["--features", "system-allocator"]
    return features


def copy_artifacts(messages: list[dict], deps: Path, host: Path, target_dir: Path) -> dict[str, list[str]]:
    """Keep target metadata separate from the host proc-macros loaded by rustc. Returns each package's copied files,
    relative to the package folder.

    An explicit Cargo --target keeps host build dependencies outside target_dir;
    those libraries are not part of the target SDK. Proc-macros alone run on the host.
    """
    copied: dict[str, list[str]] = {}
    for message in messages:
        if message["reason"] != "compiler-artifact":
            continue
        proc_macro = "proc-macro" in message["target"]["kind"]
        filenames = list(message["filenames"])
        # Cargo does not inventory standalone metadata emitted alongside a dylib.
        for name in message["filenames"]:
            if proc_macro or not any(file.endswith((".so", ".dll", ".dylib")) for file in message["filenames"]):
                continue
            metadata = Path(name).with_suffix(".rmeta")
            if metadata.is_file() and str(metadata) not in filenames:
                filenames.append(str(metadata))
        for name in filenames:
            file = Path(name)
            if not name.endswith(SUFFIXES):
                continue
            if proc_macro:
                if name.endswith((".so", ".dll", ".dylib", ".dll.a")):
                    shutil.copy2(file, host / file.name)
                    copied.setdefault(message["package_id"], []).append(f"{host.name}/{file.name}")
            elif file.is_relative_to(target_dir):
                shutil.copy2(file, deps / file.name)
                copied.setdefault(message["package_id"], []).append(f"{deps.name}/{file.name}")
    return copied


def api_closure(metadata: dict, roots=API_ROOTS) -> set[str]:
    """The package IDs an API-only mod's build reads: `roots` and their normal dependencies. A proc macro is a leaf,
    since its own dependencies are inside its library."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [package["id"] for package in metadata["packages"] if package["name"] in roots]
    closure = set()
    while pending:
        package_id = pending.pop()
        if package_id in closure:
            continue
        closure.add(package_id)
        if any("proc-macro" in target["kind"] for target in packages[package_id]["targets"]):
            continue
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"]
                       if any(kind["kind"] is None for kind in dependency["dep_kinds"]))
    return closure


def slices(metadata: dict, copied: dict[str, list[str]], api_route: list[str]) -> dict[str, list[str]]:
    """Inventory the runtime dynamic libraries and API metadata from Cargo's build records.
    All remaining files belong to the engine slice."""
    names = {package["id"]: package["name"] for package in metadata["packages"]}
    libraries = {crate.replace("_", "-") for crate in GAME_LIBRARIES}
    closure = api_closure(metadata)
    # Each slice unpacks to its own folder: the API route's crates must all be in `api`.
    inside = sorted(names[package_id] for package_id in closure if names.get(package_id) in libraries)
    if inside:
        sys.exit(f"the API closure holds game libraries {inside}, which the api slice cannot carry")
    dynamic = lambda file: file.endswith((".so", ".dylib", ".dll", ".dll.a"))
    runtime = [IDENTITY, *api_route] + [file for package_id, files in copied.items()
                if package_id in closure or names.get(package_id) in libraries
                for file in files if dynamic(file) and not file.startswith("host/")]
    api = [file for package_id in closure for file in copied.get(package_id, [])
           if not dynamic(file) or file.startswith("host/")] + [*API_BUILD_DATA, NATIVE_MODS_TEMPLATE]

    return {"runtime": sorted(set(runtime)), "api": sorted(set(api))}


def api_route(out: Path, link_args: list[str], official: Path | None = None) -> list[str]:
    """Build the fixed executable shared by both native routes, once for this SDK. Given `official` (the client's
    [`official_link`] folder), it registers Horizon's official parts, as the release's executable does."""
    deps = out / "deps"
    folder = out / RUNTIME
    folder.mkdir()
    common = ["--edition", "2024", "-C", "prefer-dynamic", "-C", "opt-level=3", "-L", f"dependency={deps}",
              "-L", f"dependency={out / 'host'}", "-L", f"native={out / NATIVE}"]
    origin = {"linux": ["-Wl,-rpath,$ORIGIN"], "darwin": ["-Wl,-rpath,@executable_path"]}.get(sys.platform, [])
    parts = []
    if official is not None:
        rlibs = list(official.glob("libhorizon_official-*.rlib"))
        if len(rlibs) != 1:
            sys.exit(f"expected exactly one horizon-official rlib in {official}: {rlibs}")
        parts = ["--cfg", "horizon_official", "-L", f"dependency={official}", "--extern", f"horizon_official={rlibs[0]}"]
    run(["rustc", *common, "--crate-type", "bin", "--crate-name", "horizon_native_game",
         *(flag for arg in origin + link_args for flag in ("-C", f"link-arg={arg}")),
         *extern_args(deps, "horizon_client"), *extern_args(deps, "mod_api"), *extern_args(deps, "horizon_mod_package"),
         "--extern", f"{GAME_LIBRARIES[0]}={dylib(deps, GAME_LIBRARIES[0])}", *parts,
 "-o", str(folder / EXECUTABLE), str(API_MAIN)])
    built = sorted(path.relative_to(out).as_posix() for path in folder.iterdir())
    if not {path.name for path in folder.iterdir()} <= api_route_files():
        sys.exit(f"the API route's build left {built}, not only {sorted(api_route_files())}")
    return built


def api_route_files() -> set[str]:
    """The runtime binary folder contains only the fixed executable."""
    return {EXECUTABLE}


def holder_mark() -> bytes:
    """The bytes the launcher looks for in an executable before it runs it under the holder (`holder::capable`),
    read from their one definition, `holder.rs`'s `CAPABILITY`."""
    source = ROOT / "crates/horizon-mod-package/src/holder.rs"
    found = re.search(rb'CAPABILITY: \[u8; \d+\] = \*b"([^"]+)"', source.read_bytes())
    if not found:
        sys.exit(f"{source.relative_to(ROOT)} no longer defines CAPABILITY as `[u8; N] = *b\"...\"`: "
                 "update holder_mark() in tools/prebuilt_game.py to read it")
    return found.group(1)


def holds(names: list[str], member: str) -> bool:
    """Whether the slice file list `names` holds `member`, an entry of the required inputs' `members`."""
    folder, _, name = member.rpartition("/")
    prefix = f"{folder}/" if folder else ""
    if name.startswith("@program:"):
        return f"{prefix}{name[9:]}{'.exe' if sys.platform == 'win32' else ''}" in names
    if name.startswith("@dylib:"):
        return f"{prefix}{dylib_prefix()}{name[7:]}{dylib_suffix()}" in names
    if name.startswith("@rmeta:"):
        if f"{prefix}lib{name[7:]}.rmeta" in names:
            return True
        return any(re.fullmatch(re.escape(f"{prefix}lib{name[7:]}-") + r"[0-9a-f]+\.rmeta", entry) for entry in names)
    return member in names


def dylib_prefix() -> str:
    return "" if sys.platform == "win32" else "lib"


def dylib_suffix() -> str:
    return {"win32": ".dll", "darwin": ".dylib"}.get(sys.platform, ".so")


def runtime_packages(metadata: dict, root: str) -> set[str]:
    """Normal SDK library package IDs, excluding host build/proc-macro trees."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [package["id"] for package in metadata["packages"] if package["name"] == root]
    visited = set()
    runtime = set()
    while pending:
        package_id = pending.pop()
        if package_id in visited:
            continue
        visited.add(package_id)
        if not any(any(kind in ("lib", "rlib", "dylib") for kind in target["kind"])
                   for target in packages[package_id]["targets"]):
            continue
        runtime.add(package_id)
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"]
                       if any(kind["kind"] is None for kind in dependency["dep_kinds"]))
    return runtime


def runtime_crates(metadata: dict, root: str) -> set[str]:
    runtime = runtime_packages(metadata, root)
    return {target["name"] for package in metadata["packages"] if package["id"] in runtime
            for target in package["targets"]
            if any(kind in ("lib", "rlib", "dylib") for kind in target["kind"])}


def native_links(metadata: dict, root: str) -> list[dict]:
    runtime = runtime_packages(metadata, root)
    return [{"name": package["name"], "version": package["version"], "library": package["links"]}
            for package in metadata["packages"] if package["id"] in runtime and package.get("links")]


def facade_crates(deps: Path, runtime: set[str]) -> list[str]:
    """Only runtime crates with a unique extern can be exposed by an internals facade."""
    eligible = []
    for crate in sorted(runtime):
        if dylib(deps, crate) is not None or len(list(deps.glob(f"lib{crate}-*.rmeta"))) == 1:
            eligible.append(crate)
    return eligible


def assert_externs(out: Path, eligible: list[str]) -> None:
    """Runs in the SDK build on every OS, including macOS, before publication."""
    for crate in set(eligible) | {"mod_api", "horizon_mod_package"}:
        extern_args(out / "deps", crate)


def dylib_crate_types(staged: Path) -> None:
    """Every API crate (and the wasm runtime) of the staged copy also builds as a dylib, and the game's library
    ([`GAME`]) as a dylib only: the client's `cargo build` of it beside [`OFFICIAL`] has no `--crate-type`, so it
    builds what `cargo rustc --crate-type dylib` builds for the server."""
    metadata = json.loads(run(["cargo", "metadata", "--format-version", "1"], cwd=staged))
    closure = api_closure(metadata)
    for package in metadata["packages"]:
        if package["id"] not in closure and package["name"] not in ("horizon-mod-wasm", GAME):
            continue
        if not any(target["kind"] == ["lib"] for target in package["targets"]):
            continue
        manifest = Path(package["manifest_path"])
        if not manifest.is_relative_to(staged):
            continue  # Registry dependencies are owned by Cargo, not rewritten.
        kinds = '["dylib"]' if package["name"] == GAME else '["rlib", "dylib"]'
        text = manifest.read_text()
        if "[lib]" in text:
            text = text.replace("[lib]", f'[lib]\ncrate-type = {kinds}', 1)
        else:
            text += f'\n[lib]\ncrate-type = {kinds}\n'
        manifest.write_text(text)


def compiled_once(artifacts: list[dict], target: Path) -> None:
    """Refuses a build that compiled a crate the official parts share with the game's library more than once: the
    parts would register with a copy of the hooks the game never reads, the build would pass, and Play with Friends
    would stay off in every modded game (#19384)."""
    for crate in SHARED_ONCE:
        units = [message["package_id"] for message in artifacts
                 if message["reason"] == "compiler-artifact" and message["target"]["name"] == crate
                 and any(Path(name).is_relative_to(target) for name in message["filenames"])]
        if len(units) > 1:
            sys.exit(f"the SDK build compiled {crate} {len(units)} times ({units}): the official parts must share the "
                     "game library's copy")


def official_link(artifacts: list[dict], target: Path, folder: Path) -> Path:
    """The folder the client's executable links the official parts from: every rlib this build compiled, linked or
    copied there, never shipped. rustc takes each crate the game's library holds from that library, so only the
    parts' own crates end up in the executable. Cargo names a library's `.rmeta` only (the build's `--emit=metadata`
    is in RUSTFLAGS, which Cargo's output listing reads), so the rlib is the file beside it."""
    if folder.exists():
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    for message in artifacts:
        if message["reason"] != "compiler-artifact":
            continue
        for name in message["filenames"]:
            file = Path(name).with_suffix(".rlib")
            if name.endswith((".rlib", ".rmeta")) and file.is_relative_to(target) and file.is_file() \
                    and not (folder / file.name).exists():
                try:
                    os.link(file, folder / file.name)
                except OSError:
                    shutil.copy2(file, folder / file.name)
    return folder


def build(out: Path, server: bool) -> None:
    layout = package_format(ROOT)
    # Resolved here: the staging and build steps run from the staged copy.
    target_dir = prebuilt_target()
    staged = stage(target_dir, prepare=dylib_crate_types)
    host = rustc_host()

    # One std for the game, every mod and the executable: the game links std dynamically, as mods will.
    rustflags = "-C prefer-dynamic --emit=metadata"
    # On macOS this also gives both dylibs an @rpath install name instead of an absolute build path.
    if sys.platform == "darwin":
        rustflags += " -C rpath"
    features = cargo_features(server, sys.platform)
    # Linux's executable RUNPATH doesn't cover transitive dependencies: the game itself must find
    # the wasm runtime beside it. macOS's @rpath install names use the executable's rpath.
    rpath = ["-C", "link-arg=-Wl,-rpath,$ORIGIN"] if sys.platform == "linux" else []
    # Encoded flags take precedence over RUSTFLAGS in Cargo.
    environment = {key: value for key, value in os.environ.items() if key != "CARGO_ENCODED_RUSTFLAGS"}
    # --timings writes target/prebuilt/cargo-timings/, outside the package; the workflow uploads it on its own.
    common = [*(["--timings"] if os.environ.get(TIMINGS) else []), "--target", host, "--profile", PROFILE,
              "--message-format", "json-render-diagnostics"]
    if server:
        # The dedicated server's game is horizon-client's, with no Horizon part (docs/layers.md, "What players get").
        command = ["cargo", "rustc", "--locked", "-p", GAME, "--lib", "--crate-type", "dylib", *common, *features,
                   *(["--", *rpath] if rpath else [])]
        flags = rustflags
    else:
        # The client's executable registers Horizon's official parts, as the release's does. They build in the game
        # library's own build, so features resolve once and every crate they share with it compiles once.
        # `cargo build` passes no flags to one crate: on Linux every library of the build gets the rpath.
        command = ["cargo", "build", "--locked", "-p", GAME, "-p", OFFICIAL, "--lib", *common,
                   *(f"{GAME}/{flag}" if previous == "--features" else flag
                     for previous, flag in zip([None, *features], features))]
        flags = " ".join([rustflags, *rpath])
        # The release profile's thin LTO never reaches the dylib build: a dylib root builds every crate as plain
        # objects. horizon-official, an rlib root, would make the crates only it uses bitcode only, which rustc
        # refuses with prefer-dynamic on Windows and the executable's plain link can't use anywhere.
        environment["CARGO_PROFILE_RELEASE_LTO"] = "false"
    messages = run(
        command,
        cwd=staged,
        env={**environment, "HORIZON_GAME_BUILD_ID": head(), "RUSTFLAGS": flags + " -D warnings", "CARGO_TARGET_DIR": str(target_dir)},
    )
    artifacts = list(map(json.loads, messages.splitlines()))
    compiled_once(artifacts, target_dir / host)
    metadata = json.loads(run([
        "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", host,
        *features,
    ], cwd=staged))
    # The official parts' own crates link into the executable only: the SDK ships the game's.
    official = set() if server else api_closure(metadata, (OFFICIAL,)) - api_closure(metadata, (GAME,))
    shipped = [message for message in artifacts if message.get("package_id") not in official]
    scripts = [
        message for message in shipped
        if message["reason"] == "build-script-executed"
    ]
    if out.exists():
        shutil.rmtree(out)
    deps = out / "deps"
    deps.mkdir(parents=True)
    macros = out / "host"
    macros.mkdir()
    copied = copy_artifacts(shipped, deps, macros, target_dir / host)
    eligible = facade_crates(deps, runtime_crates(metadata, GAME))
    assert_externs(out, eligible)
    native = out / NATIVE
    native.mkdir()
    # Only libraries that come with a crate (its source in Cargo's home, or its build script's output): a
    # system folder a script names (pkg-config's /usr/lib) is the player's own system's to provide.
    crates = (target_dir, Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    for script in scripts:
        # A static library a script names is already inside its crate's rlib, so inside the game.
        bundled = {lib.split("=")[-1] for lib in script["linked_libs"] if lib.startswith("static")}
        for search in script["linked_paths"]:
            folder = Path(search.split("=", 1)[-1]).resolve()
            if not any(folder.is_relative_to(root.resolve()) for root in crates):
                continue
            for file in sorted(folder.iterdir()):
                stem = file.name.removeprefix("lib").split(".")[0]
                if file.is_file() and file.name.endswith(NATIVE_SUFFIXES) and stem not in bundled:
                    shutil.copy2(file, native / file.name)
    # The commit's bytes, as the build data's are, not a checkout's (CRLF under Git for Windows).
    carried = {name: source.relative_to(ROOT).as_posix() for name, source in CARRIED.items() if source is not None}
    blobs = dict(tracked_blobs(ROOT, head(), tuple(carried.values())))
    for name, path in carried.items():
        (out / name).write_bytes(blobs[path])
    patches = carry_patches(staged, out)
    game = next(script for script in scripts if script["package_id"].split("#")[0].endswith("/horizon-client"))
    for manifest in BUILD_DATA:
        shutil.copy2(staged / manifest, out / manifest)
    output = Path(game["out_dir"]).parent / "output"
    link_args = executable_link_args(output)
    parts = None if server else official_link(artifacts, target_dir / host, target_dir / "official-link")
    sliced = slices(metadata, copied, api_route(out, link_args, parts))
    files = {str(path.relative_to(out).as_posix()): sha256(path) for path in sorted(out.rglob("*")) if path.is_file()}
    identity = {
        "package_format": layout,
        "commit": head(),
        "rustc": run(["rustc", "-vV"]).strip(),
        "target": host,
        "profile": PROFILE,
        "side": "server" if server else "client",
        "rustflags": rustflags,
        "link_args": link_args,
        "facade_crates": eligible,
        # The game crate's features: a `[[native.mixin]]` build compiles functions copied out of its source
        # under the same cfgs (function replacement).
        "game_features": sorted(next(
            message["features"] for message in artifacts
            if message["reason"] == "compiler-artifact"
            and message["package_id"].split("#")[0].endswith("/horizon-client")
            and "lib" in message["target"]["kind"]
        )),
        "registry_patches": patches,
        "patch_files": {name: digest for name, digest in files.items() if name.startswith("patches/")},
        "registry_lock": (staged / "Cargo.lock").read_text(),
        "native_links": native_links(metadata, GAME),
        # The API route: the mods library's crate name and metadata, and which files the `runtime` and `api` archives
        # hold (every other file is the `engine` archive's).
        "slices": sliced,
        "files": files,
    }
    (out / IDENTITY).write_text(json.dumps(identity, indent=2) + "\n")
    check(out)


def carry_patches(staged: Path, out: Path) -> dict:
    """Carry only game-owned registry patch crates, never the rest of the game source."""
    patches = tomllib.loads((staged / "Cargo.toml").read_text()).get("patch", {})
    for registry in patches.values():
        for spec in registry.values():
            source = staged / spec["path"]
            if source.is_symlink() or any(path.is_symlink() for path in source.rglob("*")):
                raise ValueError("game patch contains a symlink")
            source = source.resolve()
            if not source.is_relative_to(staged.resolve()):
                raise ValueError("game patch escapes the staged source")
            destination = out / "patches" / source.name
            shutil.copytree(source, destination, symlinks=True)
            if any(path.is_symlink() for path in destination.rglob("*")):
                raise ValueError("copied game patch contains a symlink")
            spec["path"] = destination.relative_to(out).as_posix()
    # This deliberately is not the game's buildable workspace manifest.
    lines = []
    for registry, entries in patches.items():
        lines.append(f"[patch.{json.dumps(registry)}]")
        for name, spec in entries.items():
            lines.append(f"{json.dumps(name)} = {{ path = {json.dumps(spec['path'])} }}")
    (out / "Cargo.toml").write_text("\n".join(lines) + "\n")
    return patches


def executable_link_args(output: Path) -> list[str]:
    """Executable policy emitted by the engine library's build script into the SDK identity."""
    return [line[len(LINK_ARG):] for line in output.read_text().splitlines() if line.startswith(LINK_ARG)]


def dylib(deps: Path, crate: str) -> Path | None:
    """A dylib carries its own metadata, so it has no `.rmeta` beside it."""
    names = {f"{prefix}{crate}.{ext}" for prefix in ("lib", "") for ext in ("so", "dll", "dylib")}
    return next((path for path in deps.iterdir() if path.name in names), None)


def extern_args(deps: Path, crate: str) -> list[str]:
    """Resolve a native SDK extern by name, refusing ambiguous metadata rather than choosing a variant."""
    library = dylib(deps, crate)
    if library is None:
        candidates = list(deps.glob(f"lib{crate}-*.rmeta")) + list(deps.glob(f"lib{crate}.rmeta"))
        if len(candidates) != 1:
            raise ValueError(f"expected exactly one SDK metadata for {crate}: {candidates}")
        library = candidates[0]
    return ["--extern", f"{crate}={library}"]


def check(out: Path) -> None:
    """Only compiler outputs and exactly inventoried patch sources may ship."""
    identity = json.loads((out / IDENTITY).read_text()) if (out / IDENTITY).is_file() else {}
    patch_files = identity.get("patch_files", {})
    links = [path for path in out.rglob("*") if path.is_symlink()]
    if links:
        sys.exit(f"prebuilt folder contains symlinks: {links}")
    for name, digest in patch_files.items():
        path = out / name
        if not name.startswith("patches/") or ".." in Path(name).parts or not path.is_file() or sha256(path) != digest:
            sys.exit(f"prebuilt patch inventory mismatch: {name}")
    stray = [
        path for path in out.rglob("*")
        if path.is_file() and not (path.parent == out and (path.name in CARRIED or path.name in BUILD_DATA))
        and str(path.relative_to(out).as_posix()) not in patch_files
        and not (path.parent == out / "deps" and path.name.endswith(SUFFIXES))
        and not (path.parent == out / "host" and path.name.endswith((".so", ".dll", ".dylib", ".dll.a")))
        and not (path.parent == out / NATIVE and path.name.endswith(NATIVE_SUFFIXES))
        and not (path.parent == out / RUNTIME and path.name in api_route_files())
    ]
    if stray:
        sys.exit(f"prebuilt folder holds files that aren't compiler outputs: {stray}")
    for crate in ("horizon_mod_wasm", "horizon_mod_host"):
        if dylib(out / "deps", crate) is None:
            sys.exit(f"prebuilt folder has no {crate.replace('_', '-')} dylib")

    commit = identity.get("commit")
    for name in BUILD_DATA:
        path = out / name
        header = json.loads(path.read_text()) if path.is_file() else {}
        header = header if isinstance(header, dict) else {}
        build = header.get("build")
        if header.get("schema") != BUILD_DATA_SCHEMA or not isinstance(build, dict) or build.get("commit") != commit:
            sys.exit(f"prebuilt {name} is missing or has no build-data header for commit {commit}")
    sliced = identity.get("slices", {})
    listed = [name for kind in ("runtime", "api") for name in sliced.get(kind, [])]
    absent = [name for name in listed if not (out / name).is_file()]
    if absent or len(listed) != len(set(listed)):
        sys.exit(f"prebuilt slices name absent files {absent} or overlap")
    lacking = [f"{kind}: {member}" for kind, members in json.loads(INPUTS.read_text())["members"].items()
               for member in members if not holds(sliced.get(kind, []), member)]
    if lacking:
        sys.exit(f"prebuilt slices lack required members: {lacking}")
    mark = holder_mark()
    if mark not in (out / RUNTIME / EXECUTABLE).read_bytes():
        sys.exit(f"{RUNTIME}/{EXECUTABLE} does not carry the holder marker {mark.decode()}")


def smoke(out: Path) -> None:
    """Build two starter mod dylibs, the second calling into the first, and install the fixed executable from OUT
    alone."""
    deps = out / "deps"
    assert_externs(out, json.loads((out / IDENTITY).read_text())["facade_crates"])

    def extern(crate: str) -> list[str]:
        return extern_args(deps, crate)

    common = ["--edition", "2024", "-C", "prefer-dynamic", "-C", "opt-level=1", "-L", f"dependency={deps}",
              "-L", f"dependency={out / 'host'}", "-L", f"native={out / NATIVE}"]
    installed = out.parent / SMOKE
    if installed.exists():
        shutil.rmtree(installed)
    installed.mkdir()
    # The game's libraries load from beside the executable (launcher/src/native_mods/prebuilt.rs `compile`).
    origin = {"linux": ["-Wl,-rpath,$ORIGIN"], "darwin": ["-Wl,-rpath,@executable_path"]}.get(sys.platform, [])
    identity = json.loads((out / IDENTITY).read_text())
    release = next(line.removeprefix("release: ") for line in identity["rustc"].splitlines() if line.startswith("release: "))
    starter = (ROOT / "crates/cargo-horizon-mod/template/native/lib.rs").read_text()
    template = (out / NATIVE_MODS_TEMPLATE).read_text()
    smoke_library = installed / f"{dylib_prefix()}smoke{dylib_suffix()}"
    # `smoke_user` calls `smoke` through its library, as a mod calls a declared dependency: rustc links it against
    # `smoke`'s library (on windows-gnu through its import library) and the loader resolves the call at start.
    mods = {
        "smoke": (SMOKE_LIBRARY_ITEM, [], ""),
        "smoke_user": (SMOKE_USER_ITEM, ["--extern", f"smoke={smoke_library}"], '\n[dependencies]\nsmoke = "^0.1"\n'),
    }
    records = []
    with tempfile.TemporaryDirectory(dir=out.parent) as scratch:
        scratch = Path(scratch)
        for mod, (item, externs, dependencies) in mods.items():
            build = f"{identity['commit']} {mod} 0.1.0 {release}"
            rows = {"NATIVE_BUILD": build, "MOD_ENTRIES": f'"{mod}" => crate::hooks,',
                    "BUILT_MODS": f'mod_api::__private::native::BuiltMod {{ id: "{mod}", version: "0.1.0", side: mod_api::Side::Both }},',
                    "MOD_SLOTS": "", "MOD_REDIRECTS": ""}
            entry = template
            for marker, value in rows.items():
                entry = entry.replace(f"/* {marker} */", value)
            source = starter.replace("{id}", mod) + item
            (scratch / f"{mod}_entry.rs").write_text(entry)
            (scratch / f"{mod}.rs").write_text(source + f'\n#[path = "{mod}_entry.rs"] mod entry;\n')
            library = installed / f"{dylib_prefix()}{mod}{dylib_suffix()}"
            run(["rustc", *common, "-L", f"dependency={installed}", "--crate-type", "dylib", "--crate-name", mod,
                 *extern("mod_api"), "--extern", f"horizon_mod_host={dylib(deps, 'horizon_mod_host')}", *externs,
                 *(flag for arg in origin for flag in ("-C", f"link-arg={arg}")),
                 "-o", str(library), str(scratch / f"{mod}.rs")])
            records.append({"file": library.name, "sha256": sha256(library), "kind": "mod", "mod": mod,
                            "identity": build})
            install_package(installed / "horizon-native-mods" / mod, mod, source, dependencies)
    shutil.copy2(out / RUNTIME / EXECUTABLE, installed / EXECUTABLE)
    (installed / "native-mods.json").write_text(json.dumps({"libraries": records}))
    if holder_mark() not in (installed / EXECUTABLE).read_bytes():
        sys.exit("smoke: holder capability missing from the executable")
    for library in runtime_libraries(out):
        shutil.copy2(library, installed / library.name)
    # The same executable as shipped, with no native libraries selected.
    api = out.parent / SMOKE_API
    if api.exists():
        shutil.rmtree(api)
    shutil.copytree(out / RUNTIME, api)
    for library in runtime_libraries(out):
        shutil.copy2(library, api / library.name)
    print(f"smoke: two starter mods, one calling the other, and the game executable built against {out} alone, "
          f"installed in {installed}")


# What `smoke` adds to the starter for `smoke_user` to call; never inlined, so the call crosses the libraries.
SMOKE_LIBRARY_ITEM = """
/// Called from `smoke_user`'s library.
#[inline(never)]
pub fn greeting() -> &'static str {
    "smoke"
}
"""
SMOKE_USER_ITEM = """
/// A call into `smoke`'s library.
pub fn greeting_length() -> usize {
    smoke::greeting().len()
}
"""


def install_package(folder: Path, mod: str, source: str, dependencies: str) -> None:
    """Mod MOD's package as the launcher leaves it beside the executable, which reads it back at start
    (horizon-mod-package `runtime::Runtime::load`): the starter manifest with its DEPENDENCIES table and its
    [native] pins, and its source."""
    manifest = (ROOT / "crates/cargo-horizon-mod/template/native/horizon-mod.toml").read_text()
    manifest = manifest.replace("{id}", mod).replace("{name}", mod) + dependencies
    mixins = re.search(r'pub const REV: &str = "([0-9a-f]{40})";',
                       (ROOT / "crates/horizon-mod-package/src/mixins_tool.rs").read_text())[1]
    game = re.search(r'pub const VERSION_ID: &str = "([^"]+)";',
                     (ROOT / "crates/horizon-core/src/version.rs").read_text())[1]
    horizon = run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip()
    manifest += f'\n[native]\ngame = "{game}"\nhorizon = "{horizon}"\nmixins = "{mixins}"\n'
    (folder / "src").mkdir(parents=True)
    (folder / "horizon-mod.toml").write_text(manifest)
    (folder / "Cargo.toml").write_text(f'[package]\nname = "{mod}"\nversion = "0.1.0"\nedition = "2024"\n')
    (folder / "src/lib.rs").write_text(source)


def runtime_libraries(out: Path) -> list[Path]:
    """What the launcher installs beside the executable (launcher/src/native_mods/prebuilt.rs `runtime_libraries`),
    by the same rule: every shared library the identity's `runtime` slice records under `deps/`, std, and on Windows
    the MinGW runtime the GNU DLLs import. So `start` fails when the record misses a library the game loads."""
    runtime = json.loads((out / IDENTITY).read_text())["slices"]["runtime"]
    libraries = [out / name for name in runtime
                 if name.startswith("deps/") and name.endswith((".so", ".dll", ".dylib"))]
    libdir = Path(run(["rustc", "--print", "target-libdir"]).strip())
    libraries += [path for path in libdir.iterdir()
                  if path.name.startswith(("libstd-", "std-")) and path.suffix in (".so", ".dll", ".dylib")]
    if sys.platform == "win32":
        gcc = shutil.which("gcc")
        if gcc is None:
            sys.exit("smoke: no gcc on PATH, so no MinGW runtime to install beside the executable")
        libraries += [Path(gcc).parent / name for name in MINGW_RUNTIME]
    return libraries


def start(out: Path) -> None:
    """Start two installations of the fixed executable (with smoke's two libraries and without mods),
    each as a dedicated server with no game data: it loads the game's libraries
    and sets up the mods' runtime, then stops at the missing prepared files. Fails on a crash, a hang or any other
    stop, so an archive whose libraries disagree (two allocators, arismoko/horizon#3877) cannot ship. On Windows it runs without
    MinGW on PATH, as on a player's computer, so a runtime DLL the launcher would not install fails here (arismoko/horizon#3521).
    A client's error must point to the launcher, which only the official parts' hooks do ([`OFFICIAL_PARTS`]), and a
    server's must not."""
    # Run as a script, so lab sits on the path beside it; tests import this module without running `start`.
    import lab

    side = json.loads((out / IDENTITY).read_text())["side"]
    for installed in (out.parent / SMOKE, out.parent / SMOKE_API):
        with tempfile.TemporaryDirectory(dir=out.parent) as scratch:
            scratch = Path(scratch)
            # Never a real player's folders, even ones this process inherited: every home is a folder of the scratch.
            homes = {key: scratch / key.lower() for key in
                     ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME")}
            for home in homes.values():
                home.mkdir()
            environment = lab.game_env(RUST_BACKTRACE="1")
            environment.update({key: str(home) for key, home in homes.items()})
            if sys.platform == "win32":
                # A player has no MinGW: Windows must find every DLL beside the executable or in the system, so a
                # folder of PATH that holds the MinGW runtime (the runner's MinGW, Git's, a toolchain's) is left out.
                environment["PATH"] = os.pathsep.join(
                    folder for folder in environment.get("PATH", "").split(os.pathsep)
                    if folder and not any((Path(folder) / name).is_file() for name in MINGW_RUNTIME))
            game = scratch / "game"
            game.mkdir()
            # A dedicated server stops at an unaccepted EULA before it reaches the prepared files (the EULA is accepted).
            (game / "eula.txt").write_text("eula=true\n")
            executable = installed / EXECUTABLE
            if sys.platform == "darwin":
                # Use the launcher's actual bundle preparation, not a second list of libraries.
                bundle_source = scratch / "bundle.rs"
                module = json.dumps(str(ROOT / "launcher/src/game_app.rs"))
                bundle_source.write_text(
                    'mod i18n { pub fn tr(s: &str) -> &str { s } }\n'
                    'mod shortcut { pub fn xml(s: &str) -> String { s.to_owned() } }\n'
                    f'#[path = {module}] mod game_app;\n'
                    'fn main() { game_app::ensure(std::path::Path::new(&std::env::args().nth(1).unwrap())).unwrap(); }')
                bundle_tool = scratch / "bundle"
                run(["rustc", "--edition=2021", str(bundle_source), "-o", str(bundle_tool)])
                run([str(bundle_tool), str(installed)])
                bundled = installed / "Horizon.app/Contents/MacOS/horizon-client"
                executable = bundled
                probe = subprocess.run([str(bundled), "--launch-api"], cwd=game, env=environment,
                                       capture_output=True, text=True, timeout=60)
                if probe.returncode != 0:
                    sys.exit(f"bundled launch API failed: {probe.stderr}")
                json.loads(probe.stdout)
            command = [str(executable), "--resources", str(scratch / "no-resources"), "--dedicated", "--nogui"]
            print("+", " ".join(command), flush=True)
            result = subprocess.run(command, cwd=game, env=environment, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, encoding="utf-8", errors="replace", timeout=300)
        print(result.stdout, end="")
        print(f"start: exit {result.returncode}")
        if result.returncode == DLL_NOT_FOUND:
            sys.exit("start: Windows found no copy of a DLL the executable or one of its libraries imports "
                     f"(0xC0000135): it is neither beside the executable in {installed} nor a system DLL")
        # The prepared-files check exits with status 1; any other status is a different stop.
        if result.returncode != 1 or NO_RESOURCES not in result.stdout or "panicked" in result.stdout:
            sys.exit(f"start: {installed.name} stopped before the prepared-files check (exit {result.returncode})")
        # A client's executable is the official game, a server's never has a Horizon part (docs/layers.md).
        if (OFFICIAL_PARTS in result.stdout) != (side == "client"):
            sys.exit(f"start: the {side} executable in {installed.name} "
                     + ("registered no official parts: Play with Friends, the heartbeat and the official server are "
                        "missing (#19384)" if side == "client" else "registered Horizon's official parts"))
        print(f"start: {installed.name} loaded the game's libraries and the mods' runtime and stopped at: "
              f"{NO_RESOURCES}")


def tar_xz(folder: Path, names: list[str], archive: Path) -> None:
    """A tar of `names` (paths under `folder`, folders included) the same on every OS and run: sorted `./` entries,
    zero times and owners, GNU format, xz-compressed with no timestamp of its own."""
    def clean(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.mtime, info.uid, info.gid, info.uname, info.gname = 0, 0, 0, "", ""
        return info
    with lzma.open(archive, "wb", format=lzma.FORMAT_XZ, check=lzma.CHECK_CRC64, preset=XZ_PRESET) as xz, \
            tarfile.open(fileobj=xz, mode="w", format=tarfile.GNU_FORMAT) as tar:
        for name in sorted(set(names)):
            arcname = "." if name == "." else "./" + name
            tar.add(folder / name, arcname=arcname, recursive=False, filter=clean)


def tree(folder: Path) -> list[str]:
    """Every path under `folder`, relative, with `/`, and `.` itself."""
    return ["."] + [path.relative_to(folder).as_posix() for path in folder.rglob("*")]


def archives(out: Path, sliced: dict[str, list[str]]) -> dict[str, list[str]]:
    """Each archive's entries: the identity's `runtime` and `api` files, and every other file for `engine`, each with
    the folders above it, so the three unpacked into one folder are the whole package."""
    files = {name: kind for kind in ("runtime", "api") for name in sliced[kind]}
    every = [name for name in tree(out) if (out / name).is_file()]
    chosen = {"runtime": [], "api": [], "engine": []}
    for name in every:
        chosen[files.get(name, "engine")].append(name)
    return {kind: sorted({".", *(parent.as_posix() for name in names for parent in Path(name).parents
                                if parent != Path("."))} | set(names)) for kind, names in chosen.items()}


def pack(out: Path, dist: Path, stage_tool: Path | None = None) -> None:
    """The SDK release's assets from a checked folder OUT, into DIST: the package as its `runtime`, `api` and `engine`
    archives for its target and side ([`archives`]), the build data, and the surface export of this source."""
    check(out)
    identity = json.loads((out / IDENTITY).read_text())
    if identity.get("package_format") != package_format(ROOT) or identity.get("commit") != head():
        sys.exit(f"{out} is not this source's package (format {package_format(ROOT)}, commit {head()})")
    dist.mkdir(parents=True, exist_ok=True)
    suffix = "-server" if identity["side"] == "server" else ""
    jobs = {dist / f"{kind}-{identity['target']}{suffix}.tar.xz": names
            for kind, names in archives(out, identity["slices"]).items()}
    jobs[dist / BUILD_DATA_ARCHIVE] = [".", *BUILD_DATA]
    # Each archive keeps its single xz stream; only independent archives run concurrently.
    with ProcessPoolExecutor(max_workers=len(jobs)) as pool:
        pending = {archive: pool.submit(tar_xz, out, names, archive) for archive, names in jobs.items()}
        failures = [f"{archive.name}: {error}" for archive, future in pending.items()
                    if (error := future.exception()) is not None]
    if failures:
        sys.exit("SDK archive compression failed: " + "; ".join(failures))
    command, env = ([str(stage_tool.resolve())], None) if stage_tool is not None else surface_tool()
    started = time.monotonic()
    run(command + ["surface", str(ROOT), str(dist / SURFACE)], cwd=ROOT, env=env)
    print(f"Surface export: {time.monotonic() - started:.3f}s", flush=True)
    for asset in sorted(dist.iterdir()):
        print(f"{asset.name}: {asset.stat().st_size} bytes")


def members(archive: Path, names: tuple[str, ...]) -> dict[str, str]:
    """The SHA-256 of each of `names` that the `tar.xz` `archive` holds at its root, by name."""
    found = {}
    with tarfile.open(archive, "r:xz") as tar:
        for member in tar.getmembers():
            name = member.name.removeprefix("./")
            if name in names and member.isfile():
                found[name] = hashlib.sha256(tar.extractfile(member).read()).hexdigest()
    return found


def gather(legs: Path, dist: Path) -> None:
    """The release folder DIST from the packing legs' folders under LEGS (one per OS and side, each `pack`'s DIST):
    every slice archive once, and one `build-data.tar.xz` and `surface.json`, after checking that every copy of the
    build data is the same: each leg's `build-data.tar.xz`, and the copy inside each leg's `api` slice. A leg whose
    surface or build data differs, or two legs with the same archive, stop it before anything is signed."""
    folders = sorted(path for path in legs.iterdir() if path.is_dir())
    if not folders:
        sys.exit(f"{legs} holds no packing leg")
    dist.mkdir(parents=True, exist_ok=True)
    reference = None
    for folder in folders:
        data = folder / BUILD_DATA_ARCHIVE
        surface = folder / SURFACE
        if not data.is_file() or not surface.is_file():
            sys.exit(f"{folder.name} has no {BUILD_DATA_ARCHIVE} or {SURFACE}")
        copies = {BUILD_DATA_ARCHIVE: members(data, BUILD_DATA)}
        if set(copies[BUILD_DATA_ARCHIVE]) != set(BUILD_DATA):
            sys.exit(f"{folder.name}'s {BUILD_DATA_ARCHIVE} does not hold {', '.join(BUILD_DATA)}")
        for archive in sorted(folder.glob("api-*.tar.xz")):
            copies[archive.name] = members(archive, API_BUILD_DATA)
        seen = (hashlib.sha256(surface.read_bytes()).hexdigest(), copies[BUILD_DATA_ARCHIVE])
        if reference is None:
            reference = (folder.name, *seen)
            shutil.copy2(data, dist / BUILD_DATA_ARCHIVE)
            shutil.copy2(surface, dist / SURFACE)
        if seen[0] != reference[1]:
            sys.exit(f"{folder.name}'s {SURFACE} differs from {reference[0]}'s")
        for name, hashes in copies.items():
            differ = sorted(file for file in (BUILD_DATA if name == BUILD_DATA_ARCHIVE else API_BUILD_DATA)
                            if hashes.get(file) != reference[2][file])
            if differ:
                sys.exit(f"{folder.name}/{name}: {', '.join(differ)} differ from {reference[0]}'s build data")
        for archive in sorted(folder.glob("*.tar.xz")):
            if archive.name == BUILD_DATA_ARCHIVE:
                continue
            if (dist / archive.name).exists():
                sys.exit(f"{archive.name} comes from two legs")
            shutil.copy2(archive, dist / archive.name)
    print(f"gather: {len(folders)} legs, one build data ({', '.join(f'{name} {digest[:12]}' for name, digest in sorted(reference[2].items()))})")


def index(dist: Path, nightly: str | None, previous: tuple[str, Path] | None = None) -> None:
    """DIST/sdk.json over every asset in DIST, written and checked by the one implementation (`horizon_mod_package::sdk`).
    `previous` is the SDK published before this one and the `horizon-stage surface-diff --json` of its surface
    against DIST's, which fill the index's surface comparison."""
    commit = head()
    date = run(["git", "log", "-1", "--format=%cd", "--date=format-local:%Y-%m-%d"], cwd=ROOT,
               env={**os.environ, "TZ": "UTC"}).strip()
    run(["cargo", "run", "--locked", "-q", "-p", "xtask", "--", "sdk-index", str(dist), commit, date,
         *(["--nightly", nightly] if nightly else []),
         *(["--previous", previous[0], str(previous[1])] if previous else [])], cwd=ROOT)


def authorize(commit: str) -> None:
    """Publication requires this exact main commit's successful required push runs, not a nearby green head."""
    if __package__:
        from . import release_train as train
    else:
        import release_train as train

    if not re.fullmatch(r"[0-9a-f]{40}", commit) or head() != commit:
        sys.exit("SDK publication requires the checkout's full lowercase commit SHA")
    run(["git", "merge-base", "--is-ancestor", commit, "refs/remotes/origin/main"], cwd=ROOT)
    for workflow in ("checks", "parity-main"):
        if not train.passed_on(workflow, commit):
            sys.exit(f"SDK publication refused: {workflow} has not passed on main at {commit}")
    if os.environ.get("RELEASES_PAUSED") == "1":
        sys.exit("SDK publication refused: RELEASES_PAUSED is set")
    if blockers := train.open_blockers():
        sys.exit(f"SDK publication refused: release-blocker issues {blockers} are open")


def validate_release(dist: Path, commit: str) -> None:
    """Check every compiled slice's exact identity and inventory before signing. Never execute archive contents."""
    sys.path.insert(0, str(ROOT))
    from tools.ci.nightly import SDK_ARCHIVES, SDK_LEGS

    expected = set(SDK_ARCHIVES) | {"sdk.json"}
    actual = {path.name for path in dist.iterdir()}
    if actual != expected or any(not path.is_file() or path.is_symlink() for path in dist.iterdir()):
        sys.exit(f"SDK assets differ: missing {sorted(expected - actual)}, extra {sorted(actual - expected)}")
    index = json.loads((dist / "sdk.json").read_bytes())
    rust = (ROOT / "tools/ci/rust-version").read_text().strip()
    if (index.get("build", {}).get("commit") != commit or index.get("package_format") != package_format(ROOT)
            or index.get("toolchain", {}).get("rust") != rust or set(index.get("assets", {})) != set(SDK_ARCHIVES)):
        sys.exit("SDK index does not name this exact build, toolchain, package format and asset set")
    for name, record in index["assets"].items():
        path = dist / name
        if record["size"] != path.stat().st_size or record["sha256"] != sha256(path):
            sys.exit(f"SDK index checksum mismatch: {name}")
    patches = tomllib.loads((ROOT / "Cargo.toml").read_text()).get("patch", {})
    patch_paths = tuple(spec["path"] for entries in patches.values() for spec in entries.values())
    patch_files = {}
    for name, data in tracked_blobs(ROOT, commit, patch_paths) if patch_paths else ():
        source = next(path for path in patch_paths if name.startswith(path.rstrip("/") + "/"))
        patch_files[f"patches/{Path(source).name}/{name[len(source.rstrip('/')) + 1:]}"] = hashlib.sha256(data).hexdigest()
    for entries in patches.values():
        for spec in entries.values():
            spec["path"] = f"patches/{Path(spec['path']).name}"
    for target, suffix in SDK_LEGS:
        files = {}
        identity = None
        by_slice = {}
        manifest = None
        for kind in ("runtime", "api", "engine"):
            archive = dist / f"{kind}-{target}{suffix}.tar.xz"
            by_slice[kind] = set()
            with tarfile.open(archive, "r:xz") as tar:
                for member in tar:
                    name = member.name.removeprefix("./")
                    if member.isdir():
                        continue
                    if (not member.isfile() or name in files or name.startswith("/") or ".." in Path(name).parts
                            or "\\" in name or ":" in name):
                        sys.exit(f"{archive.name}: unsafe or duplicate member {name}")
                    by_slice[kind].add(name)
                    stream = tar.extractfile(member)
                    if name == IDENTITY:
                        if kind != "runtime":
                            sys.exit(f"{archive.name}: identity is not in the runtime slice")
                        identity = json.load(stream)
                        files[name] = None
                    else:
                        if name == "Cargo.toml":
                            manifest = tomllib.loads(stream.read().decode())
                            stream.seek(0)
                        digest = hashlib.sha256()
                        for chunk in iter(lambda: stream.read(1 << 20), b""):
                            digest.update(chunk)
                        files[name] = digest.hexdigest()
        side = "server" if suffix else "client"
        if (identity is None or identity.get("commit") != commit or identity.get("target") != target
                or identity.get("side") != side or identity.get("profile") != PROFILE
                or identity.get("package_format") != package_format(ROOT)
                or f"release: {rust}" not in identity.get("rustc", "").splitlines()
                or f"host: {target}" not in identity.get("rustc", "").splitlines()):
            sys.exit(f"SDK {target}/{side}: wrong build, host, side, profile, layout or compiler")
        if identity.get("registry_patches") != patches or identity.get("patch_files") != patch_files:
            sys.exit(f"SDK {target}/{side}: registry patches differ from the exact source commit")
        if manifest != ({"patch": patches} if patches else {}):
            sys.exit(f"SDK {target}/{side}: resolution manifest differs from the exact registry patches")
        for kind in ("runtime", "api"):
            if by_slice[kind] != set(identity.get("slices", {}).get(kind, [])):
                sys.exit(f"SDK {target}/{side}: {kind} slice membership differs from its identity")
        if {name: digest for name, digest in files.items() if name != IDENTITY} != identity.get("files"):
            sys.exit(f"SDK {target}/{side}: slice contents differ from their compiler/patch inventory")
        if any(files.get(name) != digest for name, digest in patch_files.items()):
            sys.exit(f"SDK {target}/{side}: patch bytes differ from the exact source commit")
    print(f"SDK {commit}: complete exact-build assets, host/side/toolchain and patch inventories checked")


# Windows refuses a DLL past 65,535 exported names at link time. `exports` warns past 80% of that and fails past 90%, so
# a growing export list stops a package build with room left to cut it.
EXPORT_LIMIT = 65_535
EXPORT_WARNING = EXPORT_LIMIT * 8 // 10
EXPORT_FAILURE = EXPORT_LIMIT * 9 // 10


def export_count(library: Path) -> int:
    """Windows' exported-name count from a DLL's PE export directory, readable on any OS."""
    data = library.read_bytes()
    pe = int.from_bytes(data[0x3C:0x40], "little")
    optional = pe + 24
    directories = optional + (112 if int.from_bytes(data[optional:optional + 2], "little") == 0x20B else 96)
    export_rva = int.from_bytes(data[directories:directories + 4], "little")
    sections = optional + int.from_bytes(data[pe + 20:pe + 22], "little")
    for i in range(int.from_bytes(data[pe + 6:pe + 8], "little")):
        at = sections + 40 * i
        address, raw_size, raw = (int.from_bytes(data[at + o:at + o + 4], "little") for o in (12, 16, 20))
        if address <= export_rva < address + raw_size:
            directory = raw + export_rva - address
            names = int.from_bytes(data[directory + 24:directory + 28], "little")
            return names
    sys.exit(f"{library.name} has no export directory")


def exports(out: Path) -> None:
    """Report every DLL's Windows export count (arismoko/horizon#589), the separate wasm runtime included; warn on one past 80% of
    the limit and fail, after reporting them all, on any past 90%."""
    over = []
    for library in sorted((out / "deps").glob("*.dll")):
        names = export_count(library)
        print(f"{library.name}: {names} exported names (Windows' limit is {EXPORT_LIMIT:,})")
        if names > EXPORT_WARNING:
            warning = (f"{library.name} exports {names:,} names, {EXPORT_LIMIT - names:,} short of Windows' "
                       f"{EXPORT_LIMIT:,}: cut exported names before adding points or internals (#589)")
            print(f"::warning::{warning}")
            if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
                with open(summary, "a", encoding="utf-8") as file:
                    file.write(f"- {warning}\n")
        if names > EXPORT_FAILURE:
            over.append(f"{library.name} ({names:,})")
    if over:
        sys.exit(f"past 90% of Windows' {EXPORT_LIMIT:,} exported names: {', '.join(over)}; cut exported names (#589)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("out", type=Path)
    build_parser.add_argument("--server", action="store_true", help="the dedicated server's features")
    smoke_parser = commands.add_parser("smoke")
    smoke_parser.add_argument("out", type=Path)
    start_parser = commands.add_parser("start")
    start_parser.add_argument("out", type=Path)
    exports_parser = commands.add_parser("exports")
    exports_parser.add_argument("out", type=Path)
    pack_parser = commands.add_parser("pack")
    pack_parser.add_argument("out", type=Path)
    pack_parser.add_argument("dist", type=Path)
    gather_parser = commands.add_parser("gather")
    gather_parser.add_argument("legs", type=Path)
    gather_parser.add_argument("dist", type=Path)
    index_parser = commands.add_parser("index")
    index_parser.add_argument("dist", type=Path)
    index_parser.add_argument("--nightly", help="the Nightly name that ships this build")
    index_parser.add_argument("--previous", nargs=2, metavar=("BUILD", "DIFF"),
                              help="the SDK published before this one, and the surface diff against it")
    authorize_parser = commands.add_parser("authorize")
    authorize_parser.add_argument("commit")
    validate_parser = commands.add_parser("validate-release")
    validate_parser.add_argument("dist", type=Path)
    validate_parser.add_argument("commit")
    args = parser.parse_args()
    if args.command == "authorize":
        authorize(args.commit)
        return
    if args.command == "validate-release":
        validate_release(args.dist.resolve(), args.commit)
        return
    if args.command == "build":
        build(args.out.resolve(), args.server)
    elif args.command == "exports":
        exports(args.out.resolve())
    elif args.command == "start":
        start(args.out.resolve())
    elif args.command == "pack":
        pack(args.out.resolve(), args.dist.resolve())
    elif args.command == "gather":
        gather(args.legs.resolve(), args.dist.resolve())
    elif args.command == "index":
        index(args.dist.resolve(), args.nightly,
              (args.previous[0], Path(args.previous[1]).resolve()) if args.previous else None)
    else:
        smoke(args.out.resolve())


if __name__ == "__main__":
    main()
