import contextlib
import io
import json
import os
import shutil
import sys
import struct
import subprocess
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch
from pathlib import Path

from tools import prebuilt_game
from tests.test_ci_concurrency import yaml
from tests.symlinks import symlink_or_skip

COMMIT = "c" * 40


def write_build_data(root: Path, identity: dict | None = None, commit: str = COMMIT) -> None:
    """The build-data files with their header for `commit`, and an identity naming it, with the smallest
    slices `check` accepts (each required member, as an empty file) unless `identity` gives its own."""
    for name in prebuilt_game.BUILD_DATA:
        (root / name).write_text(json.dumps({"schema": 1, "build": {"commit": commit}}))
    so, exe = prebuilt_game.dylib_suffix(), ".exe" if sys.platform == "win32" else ""
    lib = prebuilt_game.dylib_prefix()
    members = {"runtime": [f"bin/horizon-client{exe}",
                           f"deps/{lib}horizon_mod_host{so}", f"deps/{lib}horizon_mod_wasm{so}",
                           *[f"deps/{lib}{crate}{so}" for crate in ("mod_api", "mod_vocab", "mod_points")]],
               "api": [prebuilt_game.NATIVE_MODS_TEMPLATE, "deps/libmod_api-1.rmeta", "deps/libmod_points-1.rmeta"]}
    for name in members["runtime"] + members["api"]:
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_bytes(b"")
    (root / prebuilt_game.RUNTIME / prebuilt_game.EXECUTABLE).write_bytes(b"\0" + prebuilt_game.holder_mark() + b"\0")
    slices = {"runtime": [prebuilt_game.IDENTITY, *members["runtime"]],
              "api": [*prebuilt_game.API_BUILD_DATA, *members["api"]]}
    (root / prebuilt_game.IDENTITY).write_text(json.dumps({"slices": slices, **(identity or {}), "commit": commit}))


class StageToolTests(unittest.TestCase):
    def test_prebuilt_tool_stages_fresh_tracked_copy_without_cargo(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            committed = b"fn first() {}\nfn second() {}\n"
            (source / "tracked.rs").write_bytes(committed)
            manifest = source / "crates/horizon-mod-wasm/Cargo.toml"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("[package]\nname = 'fixture'\n")
            git = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.autocrlf=false"]
            subprocess.run([*git, "init", "-q"], cwd=source, check=True)
            subprocess.run([*git, "add", "tracked.rs", "crates"], cwd=source, check=True)
            subprocess.run([*git, "commit", "-qm", "fixture"], cwd=source, check=True)
            commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=source, check=True, text=True,
                                    stdout=subprocess.PIPE).stdout.strip()
            # A Windows checkout's line endings, and edits never committed: neither reaches the stage.
            (source / "tracked.rs").write_bytes(committed.replace(b"\n", b"\r\n") + b"fn edited() {}\r\n")
            (source / "untracked.rs").write_text("not copied")
            target = root / "output"
            tool = root / "unused" / ".." / "horizon-stage"
            calls = []
            real_run = prebuilt_game.run

            def capture(command, **kwargs):
                if command[0] == "git":
                    return real_run(command, **kwargs)
                calls.append(command)
                fresh = target / "stage.next"
                self.assertEqual(command, [str(tool.resolve()), str(fresh),
                                          str(fresh / "points.json"),
                                          str(fresh / "internals.json"), commit])
                self.assertEqual(kwargs["cwd"], fresh)
                # Never the whole mapping in a failure message: it would print the caller's environment.
                self.assertTrue(kwargs["env"] is None, "the staged tool runs in the caller's own environment")
                self.assertEqual((fresh / "tracked.rs").read_bytes(), committed)
                self.assertFalse((fresh / "untracked.rs").exists())
                (fresh / "tracked.rs").write_text("staged")
                return "staged fixture\n"

            with patch.object(prebuilt_game, "ROOT", source), \
                 patch.object(prebuilt_game, "run", side_effect=capture):
                prebuilt_game.stage(target, stage_tool=tool)
                # A second call must replace the mutated stage with fresh tracked source.
                prebuilt_game.stage(target, stage_tool=tool)
            self.assertEqual(len(calls), 2)
            self.assertTrue((source / "tracked.rs").read_bytes().endswith(b"fn edited() {}\r\n"))
            self.assertEqual((target / "stage" / "tracked.rs").read_text(), "staged")
            self.assertFalse((target / "stage.next").exists())


class SyncUnchangedTests(unittest.TestCase):
    def test_source_paths_can_change_between_files_and_directories(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, dest = root / "source", root / "dest"
            (source / "file_to_dir").mkdir(parents=True)
            (source / "file_to_dir" / "new.rs").write_bytes(b"new nested file")
            (source / "dir_to_file").write_bytes(b"new flat file")
            (dest / "dir_to_file").mkdir(parents=True)
            (dest / "dir_to_file" / "old.rs").write_bytes(b"old nested file")
            (dest / "file_to_dir").write_bytes(b"old flat file")

            self.assertEqual(prebuilt_game.sync_unchanged(source, dest), (2, 2, 0))
            self.assertEqual((dest / "file_to_dir" / "new.rs").read_bytes(), b"new nested file")
            self.assertEqual((dest / "dir_to_file").read_bytes(), b"new flat file")

    def test_an_unchanged_file_keeps_its_mtime_and_a_changed_one_is_written(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, dest = root / "source", root / "dest"
            (source / "keep").mkdir(parents=True)
            (source / "keep" / "same.txt").write_bytes(b"same")
            (source / "new.txt").write_bytes(b"new")
            (dest / "target").mkdir(parents=True)
            (dest / "target" / "stale.o").write_bytes(b"object")
            (dest / "keep").mkdir()
            same = dest / "keep" / "same.txt"
            same.write_bytes(b"same")
            (dest / "gone.txt").write_bytes(b"gone")
            (dest / "old" / "deep").mkdir(parents=True)
            (dest / "old" / "deep" / "gone.rs").write_bytes(b"gone")
            stamp = time.time() - 120
            os.utime(same, (stamp, stamp))
            before = same.stat().st_mtime_ns
            written, removed, unchanged = prebuilt_game.sync_unchanged(source, dest)
            self.assertEqual((written, removed, unchanged), (1, 2, 1))
            self.assertEqual(same.stat().st_mtime_ns, before)
            self.assertEqual(same.read_bytes(), b"same")
            self.assertEqual((dest / "new.txt").read_bytes(), b"new")
            self.assertFalse((dest / "gone.txt").exists())
            self.assertFalse((dest / "old").exists())
            self.assertEqual((dest / "target" / "stale.o").read_bytes(), b"object")

    # A timing comparison, so it is a measurement rather than a check: a busy runner can reorder it.
    @unittest.skipUnless(os.environ.get("HORIZON_MEASURE_STAGE") == "1", "set HORIZON_MEASURE_STAGE=1 to measure")
    def test_measure_syncing_unchanged_files_against_rewriting_them(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, dest = root / "source", root / "dest"
            count = 1500
            for i in range(count):
                path = source / "crates" / f"c{i // 50}" / f"f{i}.rs"
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"fn item() {}\n" * 8)
            prebuilt_game.sync_unchanged(source, dest)

            def rewrite():
                started = time.perf_counter_ns()
                for path in source.rglob("*.rs"):
                    target = dest / path.relative_to(source)
                    target.write_bytes(path.read_bytes())
                return time.perf_counter_ns() - started

            def sync():
                started = time.perf_counter_ns()
                prebuilt_game.sync_unchanged(source, dest)
                return time.perf_counter_ns() - started

            runs = 7
            for _ in range(2):
                rewrite()
                sync()
            copied = sorted(rewrite() for _ in range(runs))
            synced = sorted(sync() for _ in range(runs))
            med = lambda samples: samples[len(samples) // 2]
            p95 = lambda samples: samples[max(0, int(len(samples) * 0.95) - 1)]
            report = (
                f"scene files={count} bytes=each about 100 runs={runs}\n"
                f"rewrite_ns median={med(copied)} p95={p95(copied)} worst={copied[-1]}\n"
                f"sync_ns median={med(synced)} p95={p95(synced)} worst={synced[-1]}\n"
            )
            Path(os.environ.get("TEMP", "."), "hz-measure-stage.txt").write_text(report)
            print(report)


class RegistryPatchTests(unittest.TestCase):
    def test_sdk_carries_only_patched_crate_and_resolution_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            staged, sdk = root / "stage", root / "sdk"
            patched = staged / "third_party" / "allocator-sys"
            patched.mkdir(parents=True)
            sdk.mkdir()
            (staged / "Cargo.toml").write_text("[workspace]\nmembers=[]\n[patch.crates-io]\nallocator-sys={path='third_party/allocator-sys'}\n")
            (patched / "Cargo.toml").write_text("[package]\nname='allocator-sys'\nversion='0.1.49'\n")
            (patched / "src").mkdir()
            (patched / "src/lib.rs").write_text("pub fn patched() {}")
            (staged / "game.rs").write_text("must not ship")
            patches = prebuilt_game.carry_patches(staged, sdk)
            self.assertEqual(patches["crates-io"]["allocator-sys"]["path"], "patches/allocator-sys")
            self.assertEqual((sdk / "patches/allocator-sys/src/lib.rs").read_text(), "pub fn patched() {}")
            self.assertFalse((sdk / "game.rs").exists())
            self.assertNotIn("workspace", (sdk / "Cargo.toml").read_text())

    def test_source_symlink_is_refused_without_copying_its_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, sdk = root / "source", root / "sdk"
            patched = source / "patch"
            patched.mkdir(parents=True)
            sdk.mkdir()
            (source / "Cargo.toml").write_text("[patch.crates-io]\nallocator-sys={path='patch'}\n")
            outside = root / "private.txt"
            outside.write_text("must not ship")
            symlink_or_skip(patched / "leak", outside)
            with self.assertRaisesRegex(ValueError, "symlink"):
                prebuilt_game.carry_patches(source, sdk)
            self.assertFalse((sdk / "patches").exists())

    def test_output_inventory_rejects_extra_files_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            sdk = Path(directory)
            patch = sdk / "patches/allocator-sys"
            patch.mkdir(parents=True)
            declared = patch / "Cargo.toml"
            declared.write_text("manifest")
            (sdk / "deps").mkdir()
            for crate in ("horizon_mod_wasm", "horizon_mod_host"):
                (sdk / "deps" / f"lib{crate}.so").write_bytes(b"")
            write_build_data(sdk, {"patch_files": {
                "patches/allocator-sys/Cargo.toml": prebuilt_game.sha256(declared)}})
            prebuilt_game.check(sdk)
            extra = patch / "extra.rs"
            extra.write_text("not inventoried")
            with self.assertRaisesRegex(SystemExit, "aren't compiler outputs"):
                prebuilt_game.check(sdk)
            extra.unlink()
            symlink_or_skip(extra, declared)
            with self.assertRaisesRegex(SystemExit, "symlinks"):
                prebuilt_game.check(sdk)
            extra.unlink()
            declared.write_text("modified")
            with self.assertRaisesRegex(SystemExit, "inventory mismatch"):
                prebuilt_game.check(sdk)


class SdkAssetTests(unittest.TestCase):
    RUNTIME = ["deps/libhorizon_mod_host.so", "deps/libhorizon_mod_wasm.so", "bin/horizon-client",
               "deps/libmod_api.so", "deps/libmod_vocab.so", "deps/libmod_points.so"]
    API = ["deps/libmod_api-1.rmeta", "deps/libmod_points-1.rmeta", "host/libserde_derive.so"]
    ENGINE = ["deps/libhorizon_client-1.rmeta", "native/libwindows.a"]

    def out(self, root: Path, mtime: int) -> Path:
        out = root / "prebuilt"
        for name in self.RUNTIME + self.API + self.ENGINE + [prebuilt_game.NATIVE_MODS_TEMPLATE]:
            (out / name).parent.mkdir(parents=True, exist_ok=True)
            (out / name).write_bytes(name.encode())
        slices = {"runtime": [prebuilt_game.IDENTITY, *self.RUNTIME],
                  "api": [*self.API, *prebuilt_game.API_BUILD_DATA, prebuilt_game.NATIVE_MODS_TEMPLATE]}
        with patch.object(sys, "platform", "linux"):
            write_build_data(out, {"package_format": 6, "side": "server", "target": "x86_64-unknown-linux-gnu",
                                   "slices": slices})
        for path in out.rglob("*"):
            os.utime(path, (mtime, mtime))
        return out

    def pack(self, out: Path, dist: Path) -> None:
        def run(command, **kwargs):
            self.assertEqual(command[-3:], ["surface", str(prebuilt_game.ROOT), str(dist / "surface.json")])
            (dist / "surface.json").write_text('{"hash": "ab"}')
        with patch.object(prebuilt_game, "head", return_value=COMMIT), patch.object(prebuilt_game, "run", side_effect=run):
            prebuilt_game.pack(out, dist, stage_tool=Path("horizon-stage"))

    def test_pack_writes_three_archives_that_split_the_package_and_the_same_every_run(self):
        import lzma
        import tarfile
        archives = []
        for mtime in (1000, 2000):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                dist = root / "dist"
                out = self.out(root, mtime)
                with patch.object(sys, "platform", "linux"):
                    self.pack(out, dist)
                names = sorted(path.name for path in dist.iterdir())
                self.assertEqual(names, ["api-x86_64-unknown-linux-gnu-server.tar.xz", "build-data.tar.xz",
                                         "engine-x86_64-unknown-linux-gnu-server.tar.xz",
                                         "runtime-x86_64-unknown-linux-gnu-server.tar.xz", "surface.json"])
                with tarfile.open(dist / "build-data.tar.xz") as tar:
                    self.assertEqual(tar.getnames(), [".", "./functions.json", "./internals.json", "./loops.json",
                                                      "./points.json"])
                files = {}
                for kind in ("runtime", "api", "engine"):
                    with lzma.open(dist / f"{kind}-x86_64-unknown-linux-gnu-server.tar.xz") as xz, \
                            tarfile.open(fileobj=xz) as tar:
                        self.assertEqual({member.mtime for member in tar.getmembers()}, {0})
                        files[kind] = sorted(m.name[2:] for m in tar.getmembers() if m.isfile())
                self.assertEqual(files["runtime"], sorted([prebuilt_game.IDENTITY, *self.RUNTIME]))
                self.assertEqual(files["api"], sorted([*self.API, *prebuilt_game.API_BUILD_DATA,
                                                      prebuilt_game.NATIVE_MODS_TEMPLATE]))
                # Everything not listed is the engine's, so the three together are the whole package, once each. The
                # function index is the engine route's alone.
                self.assertEqual(files["engine"], sorted([*self.ENGINE, prebuilt_game.FUNCTIONS]))
                every = sorted(p.relative_to(out).as_posix() for p in out.rglob("*") if p.is_file())
                self.assertEqual(sorted(files["runtime"] + files["api"] + files["engine"]), every)
                archives.append([(dist / name).read_bytes() for name in names])
        self.assertEqual(archives[0], archives[1])

    def test_parallel_pack_matches_serial_with_spawn(self):
        import multiprocessing
        from concurrent.futures import ProcessPoolExecutor
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = self.out(root, 1000)
            serial, parallel = root / "serial", root / "parallel"
            serial.mkdir()
            identity = json.loads((out / prebuilt_game.IDENTITY).read_text())
            for kind, names in prebuilt_game.archives(out, identity["slices"]).items():
                prebuilt_game.tar_xz(out, names, serial / f"{kind}-{identity['target']}-server.tar.xz")
            prebuilt_game.tar_xz(out, [".", *prebuilt_game.BUILD_DATA], serial / prebuilt_game.BUILD_DATA_ARCHIVE)
            with patch.object(prebuilt_game, "ProcessPoolExecutor", side_effect=lambda **kwargs:
                              ProcessPoolExecutor(mp_context=multiprocessing.get_context("spawn"), **kwargs)):
                self.pack(out, parallel)
            for archive in serial.iterdir():
                self.assertEqual(archive.read_bytes(), (parallel / archive.name).read_bytes(), archive.name)

    def test_worker_failure_names_archive_and_stops_surface_export(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = self.out(root, 1000)
            dist = root / "dist"
            (dist / prebuilt_game.BUILD_DATA_ARCHIVE).mkdir(parents=True)
            with self.assertRaisesRegex(SystemExit, "build-data.tar.xz"):
                self.pack(out, dist)
            self.assertFalse((dist / "surface.json").exists())

    def test_check_refuses_slices_naming_absent_or_shared_files(self):
        api = [*self.API, *prebuilt_game.API_BUILD_DATA, prebuilt_game.NATIVE_MODS_TEMPLATE]
        for change, why in (({"api": [*api, "deps/libgone-1.rmeta"]}, "absent files"),
                            ({"api": [*api, "deps/libhorizon_mod_host.so"]}, "overlap"),
                            # Each slice holds the members the required inputs name (so neither is empty).
                            ({"api": []}, "lack required members"),
                            ({"runtime": [prebuilt_game.IDENTITY, *self.RUNTIME[:2]]}, "lack required members"),
                            ({"api": api[1:]}, "lack required members"),
                            ({"api": api[:-1]}, "lack required members")):
            with tempfile.TemporaryDirectory() as directory, self.subTest(change), \
                    patch.object(sys, "platform", "linux"):
                out = self.out(Path(directory), 1000)
                identity = json.loads((out / prebuilt_game.IDENTITY).read_text())
                identity["slices"].update(change)
                (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                with self.assertRaisesRegex(SystemExit, why):
                    prebuilt_game.check(out)
        # The API executable carries the holder marker in its own bytes, or the launcher plays it the wrong way.
        with tempfile.TemporaryDirectory() as directory, patch.object(sys, "platform", "linux"):
            out = self.out(Path(directory), 1000)
            prebuilt_game.check(out)
            (out / prebuilt_game.RUNTIME / prebuilt_game.EXECUTABLE).write_bytes(b"an executable")
            with self.assertRaisesRegex(SystemExit, "holder marker"):
                prebuilt_game.check(out)

    def test_slices_come_from_the_build_records_with_proc_macros_as_leaves(self):
        def package(name, kind="lib", deps=()):
            return ({"id": name, "name": name, "targets": [{"kind": [kind]}]},
                    {"id": name, "deps": [{"pkg": dep, "dep_kinds": [{"kind": dep_kind}]} for dep, dep_kind in deps]})
        packages, nodes = zip(
            package("horizon-mod-api", deps=[("vocab", None), ("macros", None), ("tool", "build")]),
            package("horizon-mod-points", deps=[("vocab", None)]),
            package("vocab", deps=[("serde", None)]),
            package("serde"),
            package("macros", "proc-macro", deps=[("syn", None)]),
            package("syn"),
            package("tool"),
            package("horizon-mod-host", "dylib", deps=[("horizon-mod-api", None), ("engine", None)]),
            package("horizon-mod-wasm", "dylib"),
            package("engine"),
        )
        metadata = {"packages": list(packages), "resolve": {"nodes": list(nodes)}}
        self.assertEqual(prebuilt_game.api_closure(metadata),
                         {"horizon-mod-api", "horizon-mod-points", "vocab", "serde", "macros"})
        copied = {name: [f"deps/lib{name}-1.rmeta"] for name in ("horizon-mod-api", "horizon-mod-points", "vocab",
                                                                  "serde", "syn", "tool", "engine")}
        copied["macros"] = ["host/libmacros-1.so"]
        copied["horizon-mod-host"] = ["deps/libhorizon_mod_host.so"]
        copied["horizon-mod-wasm"] = ["deps/libhorizon_mod_wasm.so"]
        sliced = prebuilt_game.slices(metadata, copied, ["bin/horizon-client"])
        self.assertEqual(sliced["runtime"], sorted([prebuilt_game.IDENTITY, "bin/horizon-client",
                                                    "deps/libhorizon_mod_host.so", "deps/libhorizon_mod_wasm.so"]))
        self.assertEqual(sliced["api"], sorted([
            "deps/libhorizon-mod-api-1.rmeta", "deps/libhorizon-mod-points-1.rmeta", "deps/libvocab-1.rmeta",
            "deps/libserde-1.rmeta", "host/libmacros-1.so", *prebuilt_game.API_BUILD_DATA,
            prebuilt_game.NATIVE_MODS_TEMPLATE]))
        # Each slice unpacks to its own folder, so a game library inside the API closure is refused.
        nodes[1]["deps"].append({"pkg": "horizon-mod-wasm", "dep_kinds": [{"kind": None}]})
        with self.assertRaisesRegex(SystemExit, "horizon-mod-wasm"):
            prebuilt_game.slices(metadata, copied, ["bin/horizon-client"])

    def test_gather_takes_every_legs_archives_once_and_one_build_data_that_all_copies_match(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(sys, "platform", "linux"):
            root = Path(directory)
            legs = root / "legs"
            for side in ("server", "client"):
                out = self.out(root / side, 1000)
                identity = json.loads((out / prebuilt_game.IDENTITY).read_text())
                identity["side"] = side
                (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                self.pack(out, legs / f"sdk-linux-{side}")
            prebuilt_game.gather(legs, root / "dist")
            self.assertEqual(sorted(path.name for path in (root / "dist").iterdir()), sorted(
                [f"{kind}-x86_64-unknown-linux-gnu{suffix}.tar.xz" for kind in ("api", "engine", "runtime")
                 for suffix in ("", "-server")] + ["build-data.tar.xz", "surface.json"]))

            def refused(why, change):
                broken = root / "broken"
                shutil.rmtree(broken, ignore_errors=True)
                shutil.copytree(legs, broken)
                change(broken / "sdk-linux-server")
                with self.assertRaisesRegex(SystemExit, why):
                    prebuilt_game.gather(broken, root / "broken-dist")
                shutil.rmtree(root / "broken-dist", ignore_errors=True)

            def repack(leg, archive, name, text):
                # The archive again, with one of its files' text changed.
                folder = root / "repack"
                shutil.rmtree(folder, ignore_errors=True)
                folder.mkdir()
                with tarfile.open(leg / archive, "r:xz") as tar:
                    tar.extractall(folder, filter="data")
                (folder / name).write_text(text)
                prebuilt_game.tar_xz(folder, prebuilt_game.tree(folder), leg / archive)

            refused("surface.json differs", lambda leg: (leg / "surface.json").write_text('{"hash": "cd"}'))
            refused("points.json differ",
                    lambda leg: repack(leg, "build-data.tar.xz", prebuilt_game.POINTS, "{}"))
            # The package's own copy, in the api slice, is checked too.
            refused("api-x86_64-unknown-linux-gnu-server.tar.xz: loops.json differ",
                    lambda leg: repack(leg, "api-x86_64-unknown-linux-gnu-server.tar.xz", prebuilt_game.LOOPS, "{}"))
            refused("comes from two legs", lambda leg: shutil.copy2(
                legs / "sdk-linux-client/engine-x86_64-unknown-linux-gnu.tar.xz", leg))
            refused("no build-data.tar.xz", lambda leg: (leg / "build-data.tar.xz").unlink())

    def test_pack_exports_the_surface_with_the_tool_staging_built(self):
        # Staging builds horizon-stage in <target>/prebuilt/tools: the surface export runs that binary, and builds one
        # in the same place only when staging left none, never a second copy in <target>/release.
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "cargo-target"
            tools = (target / "prebuilt" / "tools").resolve()
            for platform, name in (("linux", "horizon-stage"), ("darwin", "horizon-stage"), ("win32", "horizon-stage.exe")):
                with self.subTest(platform=platform), patch.object(sys, "platform", platform), \
                        patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}), \
                        contextlib.redirect_stdout(io.StringIO()):
                    # No staged tool: Cargo builds it in staging's target folder.
                    command, env = prebuilt_game.surface_tool()
                    self.assertEqual(command, ["cargo", "run", "--locked", "--release", "-q", "-p",
                                               "horizon-mod-api-gen", "--bin", "horizon-stage", "--"])
                    self.assertEqual(env["CARGO_TARGET_DIR"], str(tools))
                    # Another platform's name is not this platform's tool.
                    other = tools / "release" / ("horizon-stage" if name.endswith(".exe") else "horizon-stage.exe")
                    other.parent.mkdir(parents=True, exist_ok=True)
                    other.write_bytes(b"")
                    self.assertEqual(prebuilt_game.surface_tool()[0][0], "cargo")
                    tool = tools / "release" / name
                    tool.write_bytes(b"")
                    self.assertEqual(prebuilt_game.surface_tool(), ([str(tool)], None))
                    shutil.rmtree(tools)

    def test_pack_refuses_another_commits_package(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = self.out(root, 1000)
            with patch.object(prebuilt_game, "head", return_value="d" * 40), \
                    self.assertRaisesRegex(SystemExit, "not this source's package"):
                prebuilt_game.pack(out, root / "dist")
            self.assertFalse((root / "dist").exists())

    def test_the_rust_reader_fixture_is_what_pack_writes(self):
        # crates/horizon-mod-package/src/sdk/fixture.tar.xz is unpacked by the Rust reader's test: the two agree.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "deps").mkdir()
            (root / "deps/a.rmeta").write_text("metadata")
            for path in (root / "deps", root / "deps/a.rmeta"):
                os.utime(path, (0, 0))
            prebuilt_game.tar_xz(root, prebuilt_game.tree(root), root / "fixture.tar.xz")
            fixture = prebuilt_game.ROOT / "crates/horizon-mod-package/src/sdk/fixture.tar.xz"
            self.assertEqual((root / "fixture.tar.xz").read_bytes(), fixture.read_bytes())


class TrackedBlobsTests(unittest.TestCase):
    def test_bytes_are_the_commits_whatever_the_line_ending_settings(self):
        # Git for Windows' default (core.autocrlf=true) made the Windows leg's function index differ from the others.
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            git = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-C", folder]
            subprocess.run([*git, "init", "-q"], check=True)
            (root / "a.rs").write_bytes(b"fn a() {}\nfn b() {}\n")
            subprocess.run([*git, "add", "a.rs"], check=True)
            subprocess.run([*git, "commit", "-q", "-m", "a"], check=True)
            subprocess.run([*git, "config", "core.autocrlf", "true"], check=True)
            blobs = dict(prebuilt_game.tracked_blobs(root, "HEAD"))
        self.assertEqual(blobs, {"a.rs": b"fn a() {}\nfn b() {}\n"})


class AllocatorSelectionTests(unittest.TestCase):
    def test_package_format_comes_from_its_one_place(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            number = root / "crates/horizon-mod-package/src/sdk/package-format"
            number.parent.mkdir(parents=True)
            for good in ("7\n", "7\r\n", "7"):
                number.write_bytes(good.encode())
                self.assertEqual(prebuilt_game.package_format(root), 7, repr(good))
            for broken in ("", "\n", "v7\n", "7\n8\n"):
                number.write_text(broken)
                with self.subTest(broken), self.assertRaisesRegex(ValueError, "one number"):
                    prebuilt_game.package_format(root)
        # The repository's own number is the one the archive gate compares against.
        self.assertEqual(prebuilt_game.package_format(prebuilt_game.ROOT), 6)

    def test_windows_and_macos_archives_use_the_system_allocator(self):
        for platform in ("win32", "darwin"):
            self.assertEqual(prebuilt_game.cargo_features(False, platform), ["--features", "system-allocator"])
            self.assertEqual(prebuilt_game.cargo_features(True, platform),
                             ["--no-default-features", "--features", "system-allocator"])

    def test_linux_keeps_mimalloc(self):
        self.assertEqual(prebuilt_game.cargo_features(False, "linux"), [])
        self.assertEqual(prebuilt_game.cargo_features(True, "linux"), ["--no-default-features"])

    def test_build_uses_explicit_host_target_and_target_dynamic_flags(self):
        class Captured(Exception):
            pass

        def capture(command, **kwargs):
            if command[:2] == ["cargo", "metadata"]:
                return json.dumps({"packages": [], "resolve": {"nodes": []}})
            self.assertEqual(command[command.index("--target") + 1], "fixture-host")
            self.assertEqual(kwargs["env"]["HORIZON_GAME_BUILD_ID"], "fixture-commit")
            self.assertIn("-C prefer-dynamic", kwargs["env"]["RUSTFLAGS"])
            # Single keys only: a failure must never print the caller's whole environment.
            self.assertFalse("CARGO_ENCODED_RUSTFLAGS" in kwargs["env"], "encoded flags would override RUSTFLAGS")
            commands.append(command)
            lto.append(kwargs["env"].get("CARGO_PROFILE_RELEASE_LTO"))
            raise Captured

        commands, lto = [], []
        # The timing report only when the workflow asks for one (dry runs): writing it can fail a build that compiled.
        for timings, server in ((None, False), ("1", False), (None, True)):
            with patch.object(prebuilt_game, "stage", return_value=Path("staged")), \
                 patch.object(prebuilt_game, "head", return_value="fixture-commit"), \
                 patch.object(prebuilt_game, "rustc_host", return_value="fixture-host"), \
                 patch.object(prebuilt_game, "run", side_effect=capture), \
                 patch.object(prebuilt_game.sys, "platform", "win32"), \
                 patch.dict(prebuilt_game.os.environ, {"CARGO_ENCODED_RUSTFLAGS": "-Cprefer-dynamic=no",
                                                       "CARGO_PROFILE_RELEASE_LTO": "fat-sentinel"}):
                prebuilt_game.os.environ.pop("HORIZON_SDK_TIMINGS", None)
                if timings:
                    prebuilt_game.os.environ["HORIZON_SDK_TIMINGS"] = timings
                with self.assertRaises(Captured):
                    prebuilt_game.build(Path("unused-sdk"), server)
        self.assertEqual(["--timings" in command for command in commands], [False, True, False])
        # A client's game library builds with the official parts, in one feature resolution (#19384); a server's alone.
        client, _, server = commands
        self.assertEqual(client[:2], ["cargo", "build"])
        self.assertIn(prebuilt_game.OFFICIAL, client)
        self.assertEqual(client[client.index("--features") + 1], f"{prebuilt_game.GAME}/system-allocator")
        self.assertEqual(server[:2], ["cargo", "rustc"])
        self.assertNotIn(prebuilt_game.OFFICIAL, server)
        # An rlib root would make the parts' own crates bitcode only (`-C linker-plugin-lto`), which rustc refuses with
        # prefer-dynamic on Windows; the server's dylib-only build keeps the profile as it is.
        self.assertEqual(lto, ["false", "false", "fat-sentinel"])

    def test_the_official_link_folder_takes_the_rlib_beside_the_metadata_cargo_names(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target" / "host"
            deps = target / "release/deps"
            deps.mkdir(parents=True)
            for name in ("libhorizon_official-1.rlib", "libhorizon_official-1.rmeta", "libserde-2.rlib", "libserde-2.rmeta"):
                (deps / name).write_bytes(name.encode())
            # Cargo lists only the metadata when RUSTFLAGS carries --emit=metadata; a dylib has no rlib beside it.
            artifacts = [
                {"reason": "compiler-artifact", "filenames": [str(deps / "libhorizon_official-1.rmeta")]},
                {"reason": "compiler-artifact", "filenames": [str(deps / "libserde-2.rlib"), str(deps / "libserde-2.rmeta")]},
                {"reason": "compiler-artifact", "filenames": [str(deps / "horizon_mod_host.dll")]},
                {"reason": "build-script-executed"},
            ]
            folder = prebuilt_game.official_link(artifacts, target, root / "official-link")
            self.assertEqual(sorted(path.name for path in folder.iterdir()),
                             ["libhorizon_official-1.rlib", "libserde-2.rlib"])

    def test_a_crate_shared_with_the_official_parts_compiled_twice_stops_the_build(self):
        target = Path("target-dir")

        def artifact(crate: str, unit: str) -> dict:
            return {"reason": "compiler-artifact", "package_id": unit, "target": {"name": crate, "kind": ["lib"]},
                    "filenames": [str(target / "release/deps" / f"lib{crate}-{unit}.rlib")]}

        once = [*(artifact(crate, "a") for crate in prebuilt_game.SHARED_ONCE), artifact("log", "a")]
        prebuilt_game.compiled_once(once, target)
        for crate in prebuilt_game.SHARED_ONCE:
            with self.subTest(crate), self.assertRaisesRegex(SystemExit, crate):
                prebuilt_game.compiled_once([*once, artifact(crate, "b")], target)
        # A host build of the same crate (a build script's) is not a second copy in the game.
        host = {**artifact("horizon_hooks", "h"), "filenames": [str(Path("elsewhere/libhorizon_hooks-h.rlib"))]}
        prebuilt_game.compiled_once([*once, host], target)

    def test_a_client_executable_registers_the_official_parts_and_a_server_one_does_not(self):
        for official in (False, True):
            with self.subTest(official=official), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                out = root / "out"
                (out / "deps").mkdir(parents=True)
                # The rlib as Cargo leaves it: beside the metadata its artifact record names (--emit=metadata).
                target = root / "target" / "host"
                deps = target / "release/deps"
                deps.mkdir(parents=True)
                for name in ("libhorizon_official-0123.rlib", "libhorizon_official-0123.rmeta"):
                    (deps / name).write_bytes(b"")
                record = {"reason": "compiler-artifact", "filenames": [str(deps / "libhorizon_official-0123.rmeta")]}
                link = prebuilt_game.official_link([record], target, root / "official-link")
                commands = []
                with patch.object(prebuilt_game, "extern_args", side_effect=lambda _, crate: ["--extern", crate]), \
                     patch.object(prebuilt_game, "dylib", return_value=Path("host.dll")), \
                     patch.object(prebuilt_game, "run", side_effect=lambda command, **_: commands.append(command)):
                    prebuilt_game.api_route(out, [], link if official else None)
                [command] = commands
                self.assertEqual("--cfg" in command, official)
                self.assertEqual(any(arg.startswith("horizon_official=") for arg in command), official)
                if official:
                    self.assertEqual(command[command.index("--cfg") + 1], "horizon_official")
                    self.assertIn(f"horizon_official={link / 'libhorizon_official-0123.rlib'}", command)
                    self.assertIn(f"dependency={link}", command)


class ExecutableLinkPolicyTests(unittest.TestCase):
    def test_windows_sdk_identity_keeps_the_source_executable_stack_policy(self):
        compiler = shutil.which("rustc")
        if compiler is None:
            self.skipTest("requires Rust to build and run the engine's build script")
        target = prebuilt_game.ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as directory:
            root = Path(directory)
            # Isolate unrelated code generation/resource compilation, but execute the actual
            # engine build script and its real shared link policy, as the SDK build does.
            stubs = {
                "horizon_game_discovery": 'pub fn declarations(_: &std::path::Path, _: &[&str]) '
                    '-> std::io::Result<String> { Ok(String::new()) } '
                    'pub fn folder_modules(_: &std::path::Path)->Result<String,String>{Ok(String::new())}',
                "horizon_mod_api_gen": 'pub enum Target { Client } '
                    'pub fn generate(_: &std::path::Path, _: &std::path::Path, _: Target) '
                    '-> std::io::Result<()> { Ok(()) } '
                    'pub fn mob_turn(_: &std::path::Path) -> Option<&\'static str> { None }',
                "winresource": 'pub struct WindowsResource; impl WindowsResource { '
                    'pub fn new() -> Self { Self } pub fn set_icon(&mut self, _: &str) {} '
                    'pub fn set_manifest_file(&mut self, _: &str) {} '
                    'pub fn compile(&self) -> Result<(), String> { Ok(()) } }',
            }
            externs = []
            for name, code in stubs.items():
                source = root / f"{name}.rs"
                source.write_text(code)
                library = root / f"lib{name}.rlib"
                subprocess.run([compiler, "--edition=2024", "--crate-type=rlib", str(source),
                                "-o", str(library)], check=True)
                externs.extend(["--extern", f"{name}={library}"])
            binary = root / ("producer.exe" if sys.platform == "win32" else "producer")
            subprocess.run([compiler, "--edition=2024",
                            str(prebuilt_game.ROOT / "crates/horizon-client/build.rs"),
                            *externs, "-o", str(binary)], check=True)
            for target_os, env, expected in [("windows", "msvc", ["/STACK:142606336"]),
                                      ("windows", "gnu", ["-Wl,--stack,142606336"]),
                                      ("linux", "gnu", []), ("macos", "", ["-Wl,-stack_size,0x9000000"])]:
                with self.subTest(os=target_os, env=env):
                    output = root / "output"
                    output.write_text(subprocess.check_output([str(binary)], text=True, env={
                        **os.environ,
                        "CARGO_MANIFEST_DIR": str(prebuilt_game.ROOT / "crates/horizon-client"),
                        "OUT_DIR": str(root), "CARGO_PKG_NAME": "horizon-client",
                        "CARGO_CFG_TARGET_OS": target_os, "CARGO_CFG_TARGET_ENV": env,
                    }))
                    identity = {"link_args": prebuilt_game.executable_link_args(output)}
                    self.assertEqual(identity["link_args"], expected)


class PrebuiltContentsTests(unittest.TestCase):
    def folder(self, *names: str) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        for name in names:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_bytes(b"")
        return root

    def test_only_target_artifacts_and_host_proc_macros_enter_the_archive(self):
        root = self.folder(
            "cache/release/deps/libshared-host.rmeta",
            "cache/release/deps/libderive.so",
            "cache/host/release/deps/libshared-target.rmeta",
            "cache/host/release/deps/libshared-second-version.rmeta",
            "cache/host/release/deps/libshared-target.rlib",
            "cache/host/release/deps/libhorizon_client.so",
        )
        output, macros = root / "sdk/deps", root / "sdk/host"
        output.mkdir(parents=True)
        macros.mkdir()
        def artifact(path, kind):
            return {"reason": "compiler-artifact", "target": {"kind": [kind]}, "package_id": Path(path).name,
                    "filenames": [str(root / path)]}
        copied = prebuilt_game.copy_artifacts([
            artifact("cache/release/deps/libshared-host.rmeta", "lib"),
            artifact("cache/release/deps/libderive.so", "proc-macro"),
            artifact("cache/host/release/deps/libshared-target.rmeta", "lib"),
            artifact("cache/host/release/deps/libshared-second-version.rmeta", "lib"),
            artifact("cache/host/release/deps/libshared-target.rlib", "lib"),
            artifact("cache/host/release/deps/libhorizon_client.so", "dylib"),
            {"reason": "build-finished", "success": True},
        ], output, macros, root / "cache/host")
        self.assertEqual({path.name for path in output.iterdir()}, {
            "libshared-target.rmeta", "libshared-second-version.rmeta", "libhorizon_client.so",
        })
        self.assertEqual({path.name for path in macros.iterdir()}, {"libderive.so"})
        # Each copied file is recorded under its package, as the slices read them.
        self.assertEqual(copied, {"libderive.so": ["host/libderive.so"],
                                  "libshared-target.rmeta": ["deps/libshared-target.rmeta"],
                                  "libshared-second-version.rmeta": ["deps/libshared-second-version.rmeta"],
                                  "libhorizon_client.so": ["deps/libhorizon_client.so"]})
        # Multiple legitimate target versions remain available to rustc's transitive lookup.
        # Only explicit extern/facade lookup must reject ambiguous names.
        with self.assertRaises(ValueError):
            prebuilt_game.extern_args(output, "shared")

    def test_dylib_metadata_is_collected_from_the_rlib_sidecar(self):
        root = self.folder("cache/host/release/deps/libmod_api.rlib",
                           "cache/host/release/deps/libmod_api.rmeta",
                           "cache/host/release/libmod_api.so")
        deps, host = root / "sdk/deps", root / "sdk/host"
        deps.mkdir(parents=True)
        host.mkdir()
        copied = prebuilt_game.copy_artifacts([{
            "reason": "compiler-artifact", "target": {"kind": ["lib"]}, "package_id": "api",
            "filenames": [str(root / "cache/host/release/deps/libmod_api.rlib"),
                          str(root / "cache/host/release/libmod_api.so")],
        }], deps, host, root / "cache/host")
        self.assertEqual(set(copied["api"]), {"deps/libmod_api.so", "deps/libmod_api.rmeta"})

    def test_facades_exclude_build_only_and_ambiguous_runtime_crates(self):
        def package(name, kind="lib"):
            return {"id": name, "name": name, "targets": [{"name": name.replace('-', '_'), "kind": [kind]}]}

        def node(name, *dependencies):
            return {"id": name, "deps": [
                {"pkg": target, "dep_kinds": [{"kind": kind}]} for target, kind in dependencies
            ]}

        metadata = {
            "packages": [package("horizon-mod-host", "dylib"), package("horizon-client"),
                         package("horizon-mod-wasm", "rlib"), package("point_shape"), package("horizon_core"),
                         package("build_tool"), package("derive", "proc-macro"), package("macro_helper")],
            "resolve": {"nodes": [
                node("horizon-mod-host", ("horizon-client", None), ("horizon-mod-wasm", None),
                     ("build_tool", "build")), node("horizon-mod-wasm"),
                node("horizon-client", ("point_shape", None), ("horizon_core", None), ("derive", None)),
                node("point_shape"), node("horizon_core"), node("build_tool"),
                node("derive", ("macro_helper", None)), node("macro_helper"),
            ]},
        }
        runtime = prebuilt_game.runtime_crates(metadata, "horizon-mod-host")
        self.assertEqual(runtime, {"horizon_mod_host", "horizon_mod_wasm", "horizon_client", "point_shape", "horizon_core"})
        deps = self.folder("libpoint_shape-host.rmeta", "libpoint_shape-target.rmeta",
                           "libhorizon_core-one.rmeta", "libbuild_tool-one.rmeta", "libmacro_helper-one.rmeta",
                           "libhorizon_mod_host.so", "libhorizon_mod_wasm-one.rmeta", "libhorizon_client-one.rmeta")
        eligible = prebuilt_game.facade_crates(deps, runtime)
        self.assertEqual(eligible, ["horizon_client", "horizon_core", "horizon_mod_host", "horizon_mod_wasm"])

    def test_native_link_inventory_uses_package_identity_not_a_same_named_host_variant(self):
        def package(package_id, version, library=None):
            return {"id": package_id, "name": "native-sys" if library else "horizon-mod-host",
                    "version": version, "links": library,
                    "targets": [{"name": "native_sys" if library else "horizon_client", "kind": ["lib"]}]}
        metadata = {
            "packages": [package("game", "1.0.0"), package("target", "1.0.0", "native"),
                         package("host", "2.0.0", "native")],
            "resolve": {"nodes": [
                {"id": "game", "deps": [{"pkg": "target", "dep_kinds": [{"kind": None}]},
                                          {"pkg": "host", "dep_kinds": [{"kind": "build"}]}]},
                {"id": "target", "deps": []}, {"id": "host", "deps": []},
            ]},
        }
        self.assertEqual(prebuilt_game.native_links(metadata, "horizon-mod-host"), [
            {"name": "native-sys", "version": "1.0.0", "library": "native"},
        ])

    def test_sdk_assertion_refuses_a_duplicate_eligible_facade_on_every_os(self):
        root = self.folder("deps/libhorizon_core-one.rmeta", "deps/libmod_api-one.rmeta",
                           "deps/libhorizon_mod_package-one.rmeta")
        prebuilt_game.assert_externs(root, ["horizon_core"])
        (root / "deps/libhorizon_core-two.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "expected exactly one SDK metadata for horizon_core"):
            prebuilt_game.assert_externs(root, ["horizon_core"])

    def test_native_extern_lookup_refuses_a_second_package_variant(self):
        root = self.folder("libhorizon_mod_package-runtime.rmeta")
        self.assertEqual(prebuilt_game.extern_args(root, "horizon_mod_package"), [
            "--extern", f"horizon_mod_package={root / 'libhorizon_mod_package-runtime.rmeta'}",
        ])
        (root / "libhorizon_mod_package-build.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "expected exactly one SDK metadata for horizon_mod_package"):
            prebuilt_game.extern_args(root, "horizon_mod_package")

    def test_host_macros_pass_but_host_dependency_metadata_is_refused(self):
        root = self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so", "deps/libhorizon_mod_host.so",
                           "host/libderive.so", "host/derive.dll", "host/libderive.dylib")
        write_build_data(root)
        prebuilt_game.check(root)
        (root / "host/libshared-host.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(SystemExit, "aren't compiler outputs"):
            prebuilt_game.check(root)

    def test_unrelated_multiversion_dependencies_do_not_break_named_externs(self):
        root = self.folder("deps/libmod_api-one.rmeta", "deps/libhorizon_mod_package-one.rmeta",
                           "deps/libshared-v1.rmeta", "deps/libshared-v2.rmeta",
                           "host/libmod_api-host.so")
        prebuilt_game.assert_externs(root, [])

    def test_compiler_outputs_pass(self):
        root = self.folder(
            "horizon-prebuilt.json", "Cargo.lock", "native_main.rs.template", "deps/libhorizon_client.so", "deps/libmod_api-1.rmeta",
            "native/libwindows.0.52.0.a",
            "deps/libserde_derive-2.so", "deps/libhorizon_client.dll.a", "deps/libhorizon_mod_wasm.so",
            "deps/horizon_mod_wasm.dll", "deps/libhorizon_mod_wasm.dylib", "deps/libhorizon_mod_host.so",
        )
        write_build_data(root)
        prebuilt_game.check(root)

    def test_build_data_without_its_header_for_this_commit_is_refused(self):
        root = self.folder("deps/libhorizon_mod_host.so", "deps/libhorizon_mod_wasm.so")
        for name in prebuilt_game.BUILD_DATA:
            for broken in ('{"build": {"commit": "%s"}}' % COMMIT, '{"schema": 2, "build": {"commit": "%s"}}' % COMMIT,
                           '{"schema": 1, "build": {"commit": "%s"}}' % ("d" * 40), '{"schema": 1}', "[]", None):
                with self.subTest(name=name, broken=broken):
                    write_build_data(root)
                    if broken is None:
                        (root / name).unlink()
                    else:
                        (root / name).write_text(broken)
                    with self.assertRaisesRegex(SystemExit, name):
                        prebuilt_game.check(root)

    def test_anything_else_is_refused(self):
        for stray in ("assets/minecraft/lang/en_us.json", "deps/Cargo.lock", "deps/client.jar", "deps/libmod_api-3.rlib", "native/libfoo.so", "deps/internals.json", "notes.txt"):
            with self.subTest(stray), self.assertRaises(SystemExit):
                prebuilt_game.check(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so", stray))

    def test_a_folder_without_either_runtime_is_refused(self):
        for present, missing in (("horizon_mod_host", "horizon-mod-wasm"), ("horizon_mod_wasm", "horizon-mod-host")):
            with self.subTest(present), self.assertRaisesRegex(SystemExit, missing):
                prebuilt_game.check(self.folder(f"deps/lib{present}.so"))

    def test_host_carries_engine_code_with_engine_metadata_for_mods(self):
        root = self.folder("deps/libhorizon_mod_host.so", "deps/libhorizon_mod_wasm.so", "deps/libhorizon_client-123.rmeta")
        write_build_data(root)
        prebuilt_game.check(root)

    def test_engine_and_wasm_without_host_are_refused(self):
        with self.assertRaisesRegex(SystemExit, "horizon-mod-host"):
            prebuilt_game.check(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so"))

    def dll(self, root: Path, name: str, count: int, pe64: bool = True) -> None:
        # One PE section: RVA 0x1000 maps to raw offset 0x200, holding the export directory.
        data = bytearray(0x400)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<H", data, 0x86, 1)  # NumberOfSections
        optional_size = 240 if pe64 else 224
        struct.pack_into("<H", data, 0x94, optional_size)
        struct.pack_into("<H", data, 0x98, 0x20B if pe64 else 0x10B)
        struct.pack_into("<II", data, 0x98 + (112 if pe64 else 96), 0x1000, 40)
        struct.pack_into("<III", data, 0x98 + optional_size + 12, 0x1000, 0x200, 0x200)
        struct.pack_into("<I", data, 0x200 + 24, count)  # NumberOfNames
        (root / "deps" / name).write_bytes(data)

    def test_exports_reports_every_dll(self):
        root = self.folder("deps/libhorizon_client.so")
        self.dll(root, "horizon_client.dll", 45000)
        self.dll(root, "horizon_mod_wasm.dll", 12000)
        self.dll(root, "serde_derive.dll", 1, pe64=False)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(root)
        for name, count in (("horizon_client", 45000), ("horizon_mod_wasm", 12000), ("serde_derive", 1)):
            self.assertIn(f"{name}.dll: {count} exported names", output.getvalue())

    def test_exports_warns_past_80_percent_without_failing(self):
        for large in ("horizon_client", "horizon_mod_wasm"):
            root = self.folder("deps/libhorizon_client.so")
            for name in ("horizon_client", "horizon_mod_wasm"):
                self.dll(root, f"{name}.dll", 52429 if name == large else 52428)
            output = io.StringIO()
            with self.subTest(large), contextlib.redirect_stdout(output):
                prebuilt_game.exports(root)
            warnings = [line for line in output.getvalue().splitlines() if line.startswith("::warning::")]
            self.assertEqual(len(warnings), 1)
            self.assertIn(f"{large}.dll exports 52,429 names", warnings[0])
            self.assertEqual(output.getvalue().count("exported names (Windows' limit"), 2)

    def test_exports_fails_past_90_percent_after_reporting_every_dll(self):
        root = self.folder("deps/libhorizon_client.so")
        self.dll(root, "horizon_client.dll", 58982)
        self.dll(root, "horizon_mod_wasm.dll", 58981)
        self.dll(root, "mod_api.dll", 171)
        output = io.StringIO()
        with contextlib.redirect_stdout(output), self.assertRaises(SystemExit) as failed:
            prebuilt_game.exports(root)
        self.assertIn("horizon_client.dll (58,982)", str(failed.exception.code))
        self.assertNotIn("horizon_mod_wasm", str(failed.exception.code))
        self.assertEqual(output.getvalue().count("exported names (Windows' limit"), 3)

    def test_exports_is_silent_without_dlls(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so"))
        self.assertEqual(output.getvalue(), "")


if __name__ == "__main__":
    unittest.main()


class SmokeStartTests(unittest.TestCase):
    def test_only_expected_missing_resources_exit_is_accepted_and_homes_are_scratch(self):
        from types import SimpleNamespace
        for code in (1, 0, 2, 3221225477):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "out").mkdir()
                (root / "out" / prebuilt_game.IDENTITY).write_text(json.dumps({"side": "client"}))
                result = subprocess.CompletedProcess(
                    [], code, stdout=f"{prebuilt_game.NO_RESOURCES}\n{prebuilt_game.OFFICIAL_PARTS}: download it")
                with patch.dict(sys.modules, {"lab": SimpleNamespace(game_env=lambda **_: {"HORIZON_HOME": "/inherited"})}), \
                     patch.object(prebuilt_game.subprocess, "run", return_value=result) as run, \
                     contextlib.redirect_stdout(io.StringIO()):
                    if code == 1:
                        prebuilt_game.start(root / "out")
                    else:
                        with self.assertRaises(SystemExit):
                            prebuilt_game.start(root / "out")
                environment = run.call_args.kwargs["env"]
                for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
                    self.assertTrue(Path(environment[key]).is_relative_to(root))

    def test_a_client_without_the_official_parts_or_a_server_with_them_fails(self):
        from types import SimpleNamespace
        # Only the official parts' hooks point the missing-files error at the launcher (#19384).
        missing = f"{prebuilt_game.NO_RESOURCES}: no horizon-resources.json in /x\n"
        official = missing + f"{prebuilt_game.OFFICIAL_PARTS}: download it at https://example.org (Download)."
        for side, stdout, passes in (("client", official, True), ("client", missing, False),
                                     ("server", missing, True), ("server", official, False)):
            with self.subTest(side=side, passes=passes), tempfile.TemporaryDirectory() as directory:
                out = Path(directory) / "out"
                out.mkdir()
                (out / prebuilt_game.IDENTITY).write_text(json.dumps({"side": side}))
                result = subprocess.CompletedProcess([], 1, stdout=stdout)
                with patch.dict(sys.modules, {"lab": SimpleNamespace(game_env=lambda **_: {})}), \
                     patch.object(prebuilt_game.subprocess, "run", return_value=result), \
                     contextlib.redirect_stdout(io.StringIO()):
                    if passes:
                        prebuilt_game.start(out)
                    else:
                        with self.assertRaisesRegex(SystemExit, side):
                            prebuilt_game.start(out)


class PublicationGateTests(unittest.TestCase):
    def test_only_the_exact_green_main_sha_passes_all_publication_gates(self):
        from tools import release_train as train
        with patch.object(prebuilt_game, "head", return_value=COMMIT), \
                patch.object(prebuilt_game, "run") as git, \
                patch.object(train, "passed_on", return_value=True) as passed, \
                patch.object(train, "open_blockers", return_value=[]), \
                patch.dict(os.environ, {"RELEASES_PAUSED": ""}):
            prebuilt_game.authorize(COMMIT)
            self.assertEqual([call.args for call in passed.call_args_list],
                             [("checks", COMMIT), ("parity-main", COMMIT)])
            self.assertEqual(git.call_args.args[0],
                             ["git", "merge-base", "--is-ancestor", COMMIT, "refs/remotes/origin/main"])
            for sha in ("main", "c" * 7, "d" * 40):
                with self.subTest(sha=sha), self.assertRaisesRegex(SystemExit, "full lowercase"):
                    prebuilt_game.authorize(sha)
            for denied in ("checks", "parity-main"):
                with patch.object(train, "passed_on", side_effect=lambda workflow, _: workflow != denied), \
                        self.assertRaisesRegex(SystemExit, denied):
                    prebuilt_game.authorize(COMMIT)
            with patch.dict(os.environ, {"RELEASES_PAUSED": "1"}), \
                    self.assertRaisesRegex(SystemExit, "PAUSED"):
                prebuilt_game.authorize(COMMIT)
            with patch.object(train, "open_blockers", return_value=[42]), \
                    self.assertRaisesRegex(SystemExit, "42"):
                prebuilt_game.authorize(COMMIT)


class PublicationInventoryTests(unittest.TestCase):
    def test_real_slice_archives_bind_host_side_compiler_and_source_patch_bytes(self):
        from tools.ci.nightly import SDK_ARCHIVES, SDK_LEGS
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, dist = root / "source", root / "dist"
            source.mkdir()
            dist.mkdir()
            for name, text in {
                    "Cargo.toml": "[patch.crates-io]\nfix={path='third_party/fix'}\n",
                    "third_party/fix/Cargo.toml": "[package]\nname='fix'\nversion='0.1.0'\n",
                    "third_party/fix/src/lib.rs": "pub fn fixed() {}\n",
                    "tools/ci/rust-version": "1.99.1\n",
                    "crates/horizon-mod-package/src/sdk/package-format": "6\n"}.items():
                path = source / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            def git(*args):
                return subprocess.check_output(["git", "-C", str(source), *args], text=True).strip()
            git("init", "-q")
            git("add", ".")
            git("-c", "commit.gpgsign=false", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "-qm", "source fixture")
            commit = git("rev-parse", "HEAD")
            packages = {}
            for target, suffix in SDK_LEGS:
                out = root / f"{target}{suffix}"
                out.mkdir()
                patches = prebuilt_game.carry_patches(source, out)
                for name in ("bin/program", "deps/metadata.rmeta"):
                    (out / name).parent.mkdir(parents=True, exist_ok=True)
                    (out / name).write_bytes(name.encode())
                files = {path.relative_to(out).as_posix(): prebuilt_game.sha256(path)
                         for path in out.rglob("*") if path.is_file()}
                identity = {
                    "commit": commit, "target": target, "side": "server" if suffix else "client",
                    "profile": "release", "package_format": 6,
                    "rustc": f"release: 1.99.1\nhost: {target}",
                    "registry_patches": patches,
                    "patch_files": {name: digest for name, digest in files.items() if name.startswith("patches/")},
                    "files": files,
                    "slices": {"runtime": [prebuilt_game.IDENTITY, "bin/program"], "api": ["deps/metadata.rmeta"]},
                }
                (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                packages[target, suffix] = out
            def repack(target, suffix):
                out = packages[target, suffix]
                identity = json.loads((out / prebuilt_game.IDENTITY).read_bytes())
                for kind, names in prebuilt_game.archives(out, identity["slices"]).items():
                    prebuilt_game.tar_xz(out, names, dist / f"{kind}-{target}{suffix}.tar.xz")
            for target, suffix in SDK_LEGS:
                repack(target, suffix)
            for name in ("api-source.tar.xz", "build-data.tar.xz", "surface.json"):
                (dist / name).write_bytes(name.encode())
            def reindex():
                index = {"build": {"commit": commit}, "package_format": 6, "toolchain": {"rust": "1.99.1"},
                         "assets": {name: {"sha256": prebuilt_game.sha256(dist / name),
                                          "size": (dist / name).stat().st_size} for name in SDK_ARCHIVES}}
                (dist / "sdk.json").write_text(json.dumps(index))
            target, suffix = SDK_LEGS[0]
            out = packages[target, suffix]
            original = (out / prebuilt_game.IDENTITY).read_bytes()
            with patch.object(prebuilt_game, "ROOT", source):
                reindex()
                prebuilt_game.validate_release(dist, commit)
                asset = dist / "surface.json"
                asset.write_bytes(b"changed after indexing")
                with self.assertRaisesRegex(SystemExit, "checksum mismatch"):
                    prebuilt_game.validate_release(dist, commit)
                reindex()
                (dist / "extra").write_bytes(b"extra")
                with self.assertRaisesRegex(SystemExit, "extra"):
                    prebuilt_game.validate_release(dist, commit)
                (dist / "extra").unlink()
                absent = dist / "api-source.tar.xz"
                contents = absent.read_bytes()
                absent.unlink()
                with self.assertRaisesRegex(SystemExit, "missing"):
                    prebuilt_game.validate_release(dist, commit)
                absent.write_bytes(contents)
                for field, value in (("commit", "a" * 40), ("target", "aarch64-apple-darwin"),
                                     ("side", "server"), ("rustc", "release: 1.99.0")):
                    identity = json.loads(original)
                    identity[field] = value
                    (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                    repack(target, suffix)
                    reindex()
                    with self.subTest(field=field), self.assertRaisesRegex(SystemExit, "wrong build"):
                        prebuilt_game.validate_release(dist, commit)
                (out / prebuilt_game.IDENTITY).write_bytes(original)
                changed = out / "patches/fix/src/lib.rs"
                changed.write_text("pub fn changed() {}\n")
                identity = json.loads(original)
                # Even a self-consistent compiler inventory cannot replace the committed patch bytes.
                identity["files"]["patches/fix/src/lib.rs"] = prebuilt_game.sha256(changed)
                (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                repack(target, suffix)
                reindex()
                with self.assertRaisesRegex(SystemExit, "patch bytes"):
                    prebuilt_game.validate_release(dist, commit)
