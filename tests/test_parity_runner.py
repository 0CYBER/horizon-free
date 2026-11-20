from pathlib import Path
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest import mock

from tools import parity, parity_shard
from tools.parity_shim import UNKEYED as UNKEYED_VARIABLES  # the JVM's and javac's own variables, never keyed


class ParityRunnerTests(unittest.TestCase):
    def test_dep_info_keeps_escaped_spaces(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "probe"
            Path(temp, "probe.d").write_text(f"{binary}: /a/x.rs /b/with\\ space.rs\n\n/a/x.rs:\n")
            self.assertEqual(parity.dep_info(binary), ["/a/x.rs", "/b/with space.rs"])

    def test_artifacts_follow_profile_and_stop_at_program_arguments(self):
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--release", "--example", "a", "--bin", "b"]),
                         [["example", "a", "release", [], True], ["bin", "b", "release", [], True]])
        self.assertEqual(parity.artifacts_of(["cargo", "run", "--profile", "fast", "--example", "c", "--", "--example", "x"]),
                         [["example", "c", "fast", [], True]])
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--example", "d"]), [["example", "d", "dev", [], True]])

    def test_artifacts_keep_the_package_and_feature_selection(self):
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--locked", "--profile=fast", "-j", "4", "--package",
                                              "horizon-client", "--bins", "-F", "x"]),
                         [["bins", "", "fast", ["-p", "horizon-client", "--features", "x"], False]])
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--bin=horizon-world"]),
                         [["bin", "horizon-world", "release", ["-p", "horizon-world"], False]])

    def test_artifacts_keep_combined_short_package_and_feature_flags(self):
        combined = parity.artifacts_of(["cargo", "run", "-pfoo", "-Ffeat", "--example", "p"])
        separated = parity.artifacts_of(["cargo", "run", "-p", "foo", "--features", "feat", "--example", "p"])
        self.assertEqual(combined, [["example", "p", "dev", ["-p", "foo", "--features", "feat"], True]])
        self.assertEqual(combined, separated)
        self.assertEqual(
            parity.prebuild_commands(combined, {("example", "p"): "foo"}),
            [["cargo", "build", "--locked", "--profile", "dev", "-p", "foo", "--features", "feat", "--example", "p"]],
        )

    def test_prebuild_follows_a_moved_target_without_losing_features(self):
        selection = ["-p", "horizon-client", "--no-default-features", "--features", "horizon-client/rng-trace"]
        artifacts = [["example", "client_probe", "fast", selection, True],
                     ["bin", "horizon-ctl", "fast", selection, False]]
        packages = {("example", "client_probe"): "horizon-client-lib", ("bin", "horizon-ctl"): "horizon-client-lib"}
        commands = parity.prebuild_commands(artifacts, packages)
        self.assertEqual(len(commands), 2)
        for command in commands:
            self.assertEqual(command[:5], ["cargo", "build", "--locked", "--profile", "fast"])
            self.assertEqual(command[5:5 + len(selection)], selection)
            self.assertEqual(command[5 + len(selection):7 + len(selection)], ["-p", "horizon-client-lib"])
        self.assertEqual(artifacts[0][3], selection)

    def test_old_broad_client_selections_include_the_moved_targets(self):
        packages = {("bin", "horizon-client"): "horizon-client", ("bin", "horizon-ctl"): "horizon-client-lib",
                    ("example", "client_probe"): "horizon-client-lib"}
        for flag in ["--bins", "--examples"]:
            command = ["cargo", "build", "-p", "horizon-client", "--no-default-features", flag]
            artifacts = parity.artifacts_of(command)
            before = [list(artifact[3]) for artifact in artifacts]
            self.assertEqual(parity.prebuild_commands(artifacts, packages),
                             [["cargo", "build", "--locked", "--profile", "dev", "-p", "horizon-client",
                               "--no-default-features", "-p", "horizon-client-lib", flag]])
            self.assertEqual([artifact[3] for artifact in artifacts], before)

    def test_prebuild_repeats_each_harness_selection(self):
        artifacts = [
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--example", "a"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--example", "b"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--bin", "horizon-world"]),
            *parity.artifacts_of(["cargo", "build", "--release", "--bin", "horizon-world"]),
            *parity.artifacts_of(["cargo", "run", "--profile", "fast", "-p", "horizon-client", "--example", "c"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--example", "renamed"]),
            ["example", "old", "release"],  # recorded before selections were kept: its own crate's -p
            ["example", "unknown", "release"],  # no longer in the workspace: left to its harness
        ]
        packages = {("example", "a"): "horizon-world", ("example", "b"): "horizon-world", ("example", "c"): "horizon-client",
                    ("bin", "horizon-world"): "horizon-world", ("example", "old"): "horizon-world"}
        self.assertEqual(parity.prebuild_commands(artifacts, packages), [
            ["cargo", "build", "--locked", "--profile", "fast", "-p", "horizon-client", "--example", "c"],
            ["cargo", "build", "--locked", "--profile", "release", "--bin", "horizon-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--bin", "horizon-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--example", "a", "--example", "b",
             "--example", "old"],
        ])

    def test_a_prebuild_cargo_run_says_where_its_time_went(self):
        command = ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--features", "f",
                   "--example", "a", "--example", "b", "--bins"]
        # As Cargo prints them with CARGO_TERM_COLOR=always: a bold, coloured status word, then a reset.
        lines = [(10.0, "\x1b[1m\x1b[36m    Blocking\x1b[0m waiting for file lock on package cache\n"),
                 (12.0, "\x1b[1m\x1b[36m    Blocking\x1b[0m waiting for file lock on build directory\n"),
                 (700.0, "\x1b[1m\x1b[92m   Compiling\x1b[0m serde v1.0.0\n"),
                 (900.0, "\x1b[1m\x1b[32m   Compiling\x1b[0m horizon-world v0.1.0 (/w)\n"),
                 (1000.0, "\x1b[1m\x1b[92m    Finished\x1b[0m `release` profile [optimized] target(s) in 16m 40s\n")]
        expected = ("release -p horizon-world --features f --bins 2 targets: 1001s, 2 crates compiled, "
                    "about 2s waiting for the package cache lock (to cargo's next line), "
                    "about 688s waiting for the build directory lock (to cargo's next line)")
        self.assertEqual(parity.describe_cargo_build(command, lines, 0.0, 1001.0), expected)
        plain = [(at, parity.ANSI_ESCAPE.sub("", line)) for at, line in lines]
        self.assertEqual(plain[2][1], "   Compiling serde v1.0.0\n")
        self.assertEqual(parity.describe_cargo_build(command, plain, 0.0, 1001.0), expected)
        # Still waiting when the run ended (it failed or was stopped).
        self.assertEqual(parity.describe_cargo_build(["cargo", "build", "--locked", "--example", "a"],
                                                     [(5.0, "Blocking waiting for file lock on build directory\n")],
                                                     0.0, 65.0),
                         "dev 1 target: 65s, 0 crates compiled, "
                         "about 60s waiting for the build directory lock (to cargo's next line)")

    def test_the_prebuild_still_returns_a_failed_runs_errors(self):
        # Bytes UTF-8 can't decode must not stop the line reader (and with it the build).
        script = ("import sys; print('Blocking waiting for file lock on build directory', file=sys.stderr, flush=True); "
                  "sys.stderr.buffer.write(b'\\x1b[1m\\x1b[32m   Compiling\\x1b[0m x v1 \\x81\\xff\\x81\\n'); "
                  "sys.stderr.flush(); sys.exit(1)")
        with mock.patch.object(parity, "target_packages", return_value={}), \
                mock.patch.object(parity, "prebuild_commands", return_value=[[sys.executable, "-c", script]]), \
                mock.patch.object(parity, "sccache_stats", return_value=None), \
                mock.patch.object(parity.locale, "getpreferredencoding", return_value="utf-8"), \
                mock.patch("builtins.print") as printed:
            built, errors = parity.prebuild([], 2)
        self.assertFalse(built)
        self.assertEqual(errors, "Blocking waiting for file lock on build directory\n"
                                 "\x1b[1m\x1b[32m   Compiling\x1b[0m x v1 ���\n")  # colours kept here
        self.assertIn("1 crate compiled", printed.call_args_list[0].args[0])
        # The failure path prints them where the console can't encode U+FFFD (cp1252 on Windows).
        with self.assertRaises(UnicodeEncodeError):
            print(errors, file=io.TextIOWrapper(io.BytesIO(), encoding="cp1252"))
        console = io.TextIOWrapper(io.BytesIO(), encoding="cp1252", newline="\n")
        with mock.patch.object(parity.sys, "stdout", console):
            print(parity.printable(errors))
        console.flush()
        self.assertTrue(console.buffer.getvalue().endswith(b" x v1 ???\n\n"), console.buffer.getvalue())

    def test_the_prebuild_machine_and_cache_lines(self):
        self.assertEqual(parity.describe_machine(12, (3.14, 2.0, 1.0), (9.0, 6.5, 2.25)),
                         "machine: 12 CPUs, load (1/5/15 min) at start 3.1 2.0 1.0, end 9.0 6.5 2.2")
        self.assertEqual(parity.describe_machine(8, None, None), "machine: 8 CPUs")
        self.assertEqual(parity.describe_sccache({"hits": 10, "misses": 5}, {"hits": 410, "misses": 7}),
                         "compiler cache (whole sccache server): 400 hits, 2 misses")
        for before in (None, {"hits": 500, "misses": 5}):  # none, or a server restarted meanwhile
            self.assertEqual(parity.describe_sccache(before, {"hits": 410, "misses": 7}),
                             "compiler cache: not in use, or its counts could not be read")

    def test_sccache_counts_come_from_a_file_within_a_time_limit(self):
        stats = (b"Compile requests                   500\nCache hits                         410\n"
                 b"Cache hits (Rust)                  410\nCache misses                         7\n"
                 b"Cache hits rate                  98.33 %\nCache location     /Users/\xff\n")

        class FakeSccache:
            """Popen as subprocess.run drives it; `stall` is what communicate raises before a kill."""
            started, stall = [], None
            stdin = stdout = stderr = None

            def __init__(self, command, **options):
                FakeSccache.started.append(self)
                self.args = self.command = command
                self.options, self.events = options, []
                self.returncode, self.killed = None, False
                options["stdout"].write(stats)  # a real file, not a pipe

            def __enter__(self):
                return self

            def __exit__(self, *exception):
                self.wait()

            def communicate(self, input=None, timeout=None):
                self.events.append(("communicate", timeout))
                if FakeSccache.stall and not self.killed:
                    raise FakeSccache.stall
                self.returncode = -9 if self.killed else 0
                return None, None

            def kill(self):
                self.events.append("kill")
                self.killed = True

            def wait(self, timeout=None):
                self.events.append("wait")
                self.returncode = -9 if self.killed else 0
                return self.returncode

            def poll(self):
                return self.returncode

        with mock.patch.object(parity.subprocess, "Popen", FakeSccache):
            self.assertEqual(parity.sccache_stats({"RUSTC_WRAPPER": "/opt/homebrew/bin/sccache"}),
                             {"hits": 410, "misses": 7})
            probe, = FakeSccache.started
            self.assertEqual(probe.command, ["/opt/homebrew/bin/sccache", "--show-stats"])
            self.assertNotEqual(probe.options["stdout"], subprocess.PIPE)
            self.assertEqual(probe.events[0], ("communicate", 10))
            self.assertIsNone(parity.sccache_stats({}))  # no wrapper: not asked
            self.assertEqual(len(FakeSccache.started), 1)
            # Past its 10 s: killed, then reaped (waited for) before giving up.
            FakeSccache.stall = subprocess.TimeoutExpired("sccache", 10)
            self.assertIsNone(parity.sccache_stats({"RUSTC_WRAPPER": "sccache"}))
            probe = FakeSccache.started[-1]
            self.assertEqual(probe.events[:2], [("communicate", 10), "kill"])
            self.assertTrue(set(probe.events[2:]) & {"wait", ("communicate", None)}, probe.events)
            self.assertEqual(probe.returncode, -9)
            # An interrupt still ends the run, with the probe killed.
            FakeSccache.stall = KeyboardInterrupt()
            with self.assertRaises(KeyboardInterrupt):
                parity.sccache_stats({"RUSTC_WRAPPER": "sccache"})
            self.assertIn("kill", FakeSccache.started[-1].events)
        for failure in (OSError, ValueError):
            with mock.patch.object(parity.subprocess, "Popen", side_effect=failure):
                self.assertIsNone(parity.sccache_stats({"RUSTC_WRAPPER": "sccache"}))

    def test_a_probe_binary_depends_only_on_the_probes_it_ran(self):
        commands = [["/t/release/examples/world_probe", "a_fixture", "/dir"],
                    ["cargo", "run", "-p", "horizon-client", "--example", "client_probe", "--", "b_fixture"]]
        self.assertEqual(parity.probes_run(commands), {"a_fixture", "b_fixture"})
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            probe = Path(temp, "crates/w/examples/probe")
            (probe / "support").mkdir(parents=True)
            (probe / "a_fixture.rs").write_text('mod integration { include!("d_fixture.rs"); }')
            (probe / "d_fixture.rs").write_text("use crate::support::shared as s;")
            (probe / "b_fixture.rs").write_text("use crate::e_fixture;")
            for name in ("c_fixture.rs", "dispatch.rs", "e_fixture.rs", "main.rs", "support/shared.rs", "support/other.rs"):
                (probe / name).write_text("")
            paths = [f"crates/w/examples/probe/{name}" for name in ("a_fixture.rs", "b_fixture.rs", "c_fixture.rs",
                                                                    "d_fixture.rs", "dispatch.rs", "e_fixture.rs", "main.rs",
                                                                    "support/other.rs", "support/shared.rs")]
            self.assertEqual(parity.only_probes_run(["crates/w/src/lib.rs", *paths], {"a_fixture", "b_fixture"}),
                             ["crates/w/src/lib.rs", *paths[:2], *paths[3:6], paths[8]])
            self.assertEqual(parity.only_probes_run(paths, set()), paths)

    def test_probe_names_are_each_crates_own_target(self):
        self.assertEqual(parity.probes_run([
            ["C:/t/debug/examples/client_probe.exe", "a_fixture"],
            ["cargo", "run", "--release", "--example=world_probe", "--", "b_fixture", "x"],
            ["/t/release/examples/probe", "old_fixture"],  # the shared name before 2026-10-24
            ["cargo", "run", "--example", "probe", "--", "old_fixture"],
            ["/t/release/examples/mod_port_probe", "not_a_fixture"],
            ["cargo", "run", "--example", "mod_port_probe", "--", "not_a_fixture"],
            ["cargo", "build", "--example", "world_probe"],
        ]), {"a_fixture", "b_fixture"})

    def test_records_of_the_shared_probe_name_build_their_crates_probe(self):
        record = {"dependencies": ["crates/horizon-world/examples/probe/a_fixture.rs", "crates/horizon-world/src/lib.rs"],
                  "artifacts": [["example", "probe", "release", ["-p", "horizon-client"], True],
                                ["example", "probe", "release"],  # no selection: the crate of its probe sources
                                ["example", "probe", "release", ["-p", "horizon-core", "-p", "horizon-world"], True],
                                ["bin", "horizon-world", "release", ["-p", "horizon-world"], False]]}
        artifacts = parity.recorded_artifacts(record)
        self.assertEqual(artifacts, [["example", "client_probe", "release", ["-p", "horizon-client"], True],
                                     ["example", "world_probe", "release"],
                                     ["example", "world_probe", "release", ["-p", "horizon-core", "-p", "horizon-world"], True],
                                     ["bin", "horizon-world", "release", ["-p", "horizon-world"], False]])
        packages = {("example", "client_probe"): "horizon-client", ("example", "world_probe"): "horizon-world",
                    ("bin", "horizon-world"): "horizon-world"}
        self.assertEqual(parity.prebuild_commands(artifacts, packages), [
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-client", "--example", "client_probe"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-core", "-p", "horizon-world",
             "--example", "world_probe"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--bin", "horizon-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--example", "world_probe"],
        ])
        self.assertEqual(parity.recorded_artifacts({"artifacts": [["example", "probe", "release"]]}),
                         [["example", "probe", "release"]])  # no crate to tell: left to its harness

    def test_java_sources_include_classes_pulled_from_the_sourcepath(self):
        with tempfile.TemporaryDirectory() as temp:
            sources, classes = Path(temp, "src"), Path(temp, "classes")
            (sources / "p").mkdir(parents=True)
            (classes / "p").mkdir(parents=True)
            for name in ("Oracle", "Helper"):
                (sources / "p" / f"{name}.java").write_text("")
            for name in ("Oracle", "Helper$Inner"):
                (classes / "p" / f"{name}.class").write_bytes(b"")
            found = parity.java_sources(["javac", "-sourcepath", str(sources), "-d", str(classes), str(sources / "p/Oracle.java")])
            self.assertEqual(sorted(Path(path).name for path in found), ["Helper.java", "Oracle.java", "Oracle.java"])

    def test_a_pass_goes_stale_when_a_dependency_changes(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            source = Path(temp, "a.rs")
            source.write_text("one")
            state = {"harnesses": {"x": {"status": "pass", "dependencies": ["a.rs"]}}, "hashes": {}}
            hasher = parity.Hasher(state["hashes"])
            state["harnesses"]["x"]["key"] = parity.key(state["harnesses"]["x"], hasher, "env")
            self.assertFalse(parity.stale("x", state, hasher, "env"))
            self.assertTrue(parity.stale("x", state, hasher, "other toolchain"))
            source.write_text("two!")
            self.assertTrue(parity.stale("x", state, hasher, "env"))

    def test_a_group_file_added_after_the_record_counts(self):
        # tools/command_exec_groups/ modules are found by file name (#16833): one added later is in no record.
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)),                 mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            groups = Path(temp, "tools", "command_exec_groups")
            groups.mkdir(parents=True)
            (groups / "old.py").write_text("old")
            found = {name: Path(temp, "tools", f"test_{name}_parity.py") for name in ("command_exec", "food")}
            records = {"command_exec": {"status": "pass", "dependencies": ["tools/command_exec_groups/old.py"]},
                       "food": {"status": "pass", "dependencies": ["tools/lab.py"]}}
            state = {"harnesses": records, "hashes": {}}
            hasher = parity.Hasher(state["hashes"])
            for record in records.values():
                record["key"] = parity.key(record, hasher, "env")
            self.assertFalse(parity.stale("command_exec", state, hasher, "env"))
            (groups / "new.py").write_text("new")
            self.assertTrue(parity.stale("command_exec", state, hasher, "env"))
            self.assertFalse(parity.stale("food", state, hasher, "env"))
            self.assertEqual(parity.affected_harnesses(found, records, {"tools/command_exec_groups/new.py"}),
                             {"command_exec"})

    def test_a_change_affects_the_harnesses_that_depend_on_it(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            found = {name: Path(temp, "tools", f"test_{name}_parity.py") for name in ("food", "light", "new")}
            records = {"food": {"dependencies": ["crates/a.rs", "tools/lab.py"]},
                       "light": {"dependencies": ["crates/lib.rs#keep:light"]}}
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/a.rs"}), {"food", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/lib.rs"}), {"light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"tools/test_light_parity.py"}), {"light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"Cargo.lock"}), {"food", "light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"README.md"}), {"new"})

    def test_platform_specific_harnesses_are_skipped_elsewhere(self):
        config = {"linux_only": {"options": "libm"}}
        with mock.patch.object(parity.sys, "platform", "linux"):
            self.assertEqual(parity.platform_skips(config), {})
        with mock.patch.object(parity.sys, "platform", "darwin"):
            self.assertEqual(parity.platform_skips(config), {"options": "libm"})

    def test_termination_signals_are_caught_where_they_exist(self):
        for numbers in ({"SIGTERM": 15, "SIGHUP": 1}, {"SIGTERM": 15}):  # Windows has no SIGHUP
            handled = []
            fake = SimpleNamespace(**numbers, signal=lambda number, _handler: handled.append(number))
            with self.subTest(numbers=numbers), mock.patch.object(parity, "signal", fake):
                parity.exit_on_termination()
                self.assertEqual(handled, list(numbers.values()))

    def test_remote_executor_marks_setup_failure_as_dead(self):
        executor = parity.RemoteExecutor.__new__(parity.RemoteExecutor)
        executor.name = "remote"
        executor.config = {"ssh": "host", "root": "/tmp"}
        executor.mirror = "/tmp/mirror"
        executor.dead = False
        executor._log = mock.Mock()
        with mock.patch.object(parity.subprocess, "run", side_effect=OSError("ssh not found")):
            executor._connect([])
        self.assertTrue(executor.dead)
        executor._log.assert_called_once()

    def test_remote_executor_marks_worker_start_failure_as_dead(self):
        executor = parity.RemoteExecutor.__new__(parity.RemoteExecutor)
        executor.name = "remote"
        executor.config = {"ssh": "host", "root": "/tmp"}
        executor.mirror = "/tmp/mirror"
        executor.dead = False
        executor._log = mock.Mock()
        successful = SimpleNamespace(returncode=0, stderr="")
        with mock.patch.object(parity.subprocess, "run", return_value=successful), \
                mock.patch.object(parity.subprocess, "Popen", side_effect=OSError("ssh not found")):
            executor._connect([])
        self.assertTrue(executor.dead)
        executor._log.assert_called_once()

    def test_a_hosts_excludes_are_skipped_when_it_runs_parity_itself(self):
        config = {"hosts": {"playhorizon": {"exclude": {"remote_fireworks": "starved"}}, "macmini": {"exclude": {}}}}
        with mock.patch.object(parity.sys, "platform", "linux"):
            self.assertEqual(parity.platform_skips(config, "playhorizon"), {"remote_fireworks": "starved"})
            self.assertEqual(parity.platform_skips(config, "macmini"), {})
            self.assertEqual(parity.platform_skips(config, "elsewhere"), {})

    def test_known_failing_harnesses_report_without_failing_the_run(self):
        results = {"crafter": {"status": "fail"}, "datafix#1": {"status": "fail"}, "datafix#2": {"status": "pass"},
                   "sculk": {"status": "pass"}, "mace": {"status": "fail"}, "boat_void": {"status": "pass"}}
        known = {"crafter": "#1: max_damage", "datafix": "#2: paths", "sculk": "#3: phase"}
        blocking, again, passed = parity.split_known_failing(results, known)
        self.assertEqual(blocking, ["mace"])  # not on the list: it fails the run
        self.assertEqual(again, ["crafter", "datafix#1"])  # a failing shard counts under its harness
        self.assertEqual(passed, ["sculk"])  # passes now: datafix doesn't, as one of its shards failed
        self.assertEqual(parity.split_known_failing({"crafter": {"status": "fail"}}, known)[0], [])
        # A listed shard covers only itself.
        self.assertEqual(parity.split_known_failing(results, {"datafix#1": "#2: paths"})[:2],
                         (["crafter", "mace"], ["datafix#1"]))
        self.assertEqual(parity.split_known_failing(results, {"datafix#2": "#2: paths"})[2], ["datafix#2"])
        self.assertEqual(parity.known_failing({}), {})

    def test_named_only_harnesses_are_listed_from_the_host_config(self):
        self.assertEqual(parity.named_only({"named_only": {"lan_integration": "slow"}}), {"lan_integration": "slow"})
        self.assertEqual(parity.named_only({}), {})
        self.assertIn("lan_integration", parity.named_only(parity.host_config()))

    def test_a_pass_from_another_worktree_counts_only_for_the_same_content(self):
        with tempfile.TemporaryDirectory() as temp:
            elsewhere, here, shared = Path(temp, "a"), Path(temp, "b"), Path(temp, "shared")
            for root in (elsewhere, here):
                root.mkdir()
                (root / "x.rs").write_text("same")
            with mock.patch.object(parity, "SHARED", shared):
                with mock.patch.object(parity, "ROOT", elsewhere):
                    record = {"status": "pass", "dependencies": ["x.rs"], "artifacts": []}
                    record["key"] = parity.key(record, parity.Hasher({}), "env")
                    parity.publish("x", record)
                with mock.patch.object(parity, "ROOT", here):
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertFalse(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))
                    self.assertTrue(state["harnesses"]["x"]["shared"])
                    (here / "x.rs").write_text("changed")
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertTrue(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))


class TraceTests(unittest.TestCase):
    def test_commands_are_recorded_as_argument_lists(self):
        """On Windows the audit event carries the command line Popen built; the trace splits it back, so a harness's
        cargo builds reach the probe prebuild and its probes' sources become dependencies there too."""
        arguments = [sys.executable, "-c", "pass", "build", "-p", "a b", 'say "hi"', "back\\slash\\", ""]
        with tempfile.TemporaryDirectory() as tmp:
            script, trace = Path(tmp) / "harness.py", Path(tmp) / "trace.json"
            script.write_text(f"import subprocess\nsubprocess.run({arguments!r})\n", encoding="utf-8")
            subprocess.run([sys.executable, str(Path(parity.__file__)), "--trace", str(trace), str(script)], check=True)
            commands = json.loads(trace.read_text(encoding="utf-8"))["commands"]
        self.assertIn(arguments, commands)

    @unittest.skipUnless(os.name == "nt", "Windows command lines")
    def test_a_windows_command_line_splits_into_its_arguments(self):
        for arguments in (["cargo", "build", "--profile", "fast"],
                          ["C:\\Program Files\\x.exe", "a b", 'q"uote', "tail\\", ""]):
            self.assertEqual(parity.command_line_arguments(subprocess.list2cmdline(arguments)), arguments)

    def test_a_windows_probe_binary_finds_its_dep_info(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "client_probe.d").write_text(f"{tmp}/client_probe: crates/a/src/lib.rs\n", encoding="utf-8")
            self.assertEqual(parity.dep_info(Path(tmp, "client_probe.exe")), ["crates/a/src/lib.rs"])


class DependencyPathTests(unittest.TestCase):
    def test_traced_dependencies_use_repository_posix_paths(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp).resolve()):
            source = Path(temp, "crates/w/src/lib.rs")
            source.parent.mkdir(parents=True)
            source.write_text("")
            trace = {"modules": [str(source)], "read": [], "commands": []}
            self.assertEqual(parity.dependencies(trace), (["crates/w/src/lib.rs"], []))

    def test_legacy_local_and_shared_paths_select_without_rewriting_records(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            root = Path(temp)
            found = {name: root / "tools" / f"test_{name}_parity.py" for name in ("local", "shared", "empty")}
            record = {"dependencies": [r"crates\w\src\lib.rs#keep:used"], "key": "recorded-content-key"}
            local = root / "state.json"
            local.write_text(json.dumps({"local": record, "empty": {"dependencies": []}}), encoding="utf-8")
            shared = parity.SHARED / "dependencies/shared.json"
            shared.parent.mkdir(parents=True)
            shared.write_text(json.dumps(record), encoding="utf-8")
            records = json.loads(local.read_text(encoding="utf-8"))
            before = local.read_bytes(), shared.read_bytes()
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/w/src/lib.rs"}, reach=False),
                             {"local", "shared"})
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/w/src/other.rs"}, reach=False), set())
            self.assertEqual(records, {"local": record, "empty": {"dependencies": []}})
            self.assertEqual((local.read_bytes(), shared.read_bytes()), before)

    def test_changed_probes_and_oracles_accept_legacy_separators(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            found = {"probe": Path(temp, "tools/test_probe_parity.py")}
            for path in ("crates/w/examples/probe.rs", "crates/w/src/bin/probe.rs",
                         "mods/horizon/src/test/java/Oracle.java"):
                with self.subTest(path=path), mock.patch.object(parity, "changed_files", return_value={path}):
                    records = {"probe": {"dependencies": [path.replace("/", "\\")]}}
                    self.assertEqual(parity.changed_harnesses(found, records, "main"), {"probe"})
            with mock.patch.object(parity, "changed_files", return_value={"crates/w/examples/probe.rs"}):
                records = {"probe": {"dependencies": [r"crates\w\examples\probe.rs#keep:used"]}}
                self.assertEqual(parity.changed_harnesses(found, records, "main"), {"probe"})

    def test_legacy_probe_records_still_narrow_to_reachable_sources(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")), \
                mock.patch.object(parity, "STATE_DIR", Path(temp, "state")):
            root = Path(temp)
            files = {
                "crates/horizon-world/Cargo.toml": '[package]\nname = "horizon-world"\n'
                                                   '\n[[example]]\nname = "world_probe"\npath = "examples/probe.rs"\n',
                "crates/horizon-world/src/lib.rs": "pub mod used;\npub mod unused;\n",
                "crates/horizon-world/src/used.rs": "pub fn f() {}\n",
                "crates/horizon-world/src/unused.rs": "pub fn f() {}\n",
                "crates/horizon-world/examples/probe.rs": "fn main() { horizon_world::used::f(); }\n",
            }
            for path, text in files.items():
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text, encoding="utf-8")
            dependencies = [path.replace("/", "\\") for path in files]
            dependencies[1] += "#keep:used"
            record = {"dependencies": dependencies, "artifacts": [["example", "probe", "release"]]}
            found = {"probe": root / "tools/test_probe_parity.py"}
            for path, expected in (("crates/horizon-world/src/used.rs", {"probe"}),
                                   ("crates/horizon-world/src/unused.rs", set())):
                with self.subTest(path=path):
                    self.assertEqual(parity.affected_harnesses(found, {"probe": record}, {path}), expected)
            self.assertEqual(record["dependencies"], dependencies)


class RustScopeTests(unittest.TestCase):
    """PARITY_RUST_SCOPE drops Rust sources only when nothing in the scope can reach them."""

    def crate(self, temp, files):
        root = Path(temp)
        (root / "crates/w/src").mkdir(parents=True)
        (root / "crates/w/examples").mkdir(parents=True)
        (root / "crates/w/Cargo.toml").write_text("")
        for name, text in files.items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        return root

    FILES = {
        "crates/w/src/lib.rs": "pub mod a;\npub mod b;\n/// The other one.\npub mod c;\npub const V: i32 = 1;\n",
        "crates/w/src/a.rs": 'pub struct Tag;\npub fn f() -> i32 { let s = "crate::c::x"; crate::V }\n'
                             "mod tests { fn g() { super::super::b::h(); } }\n",
        "crates/w/src/b.rs": "pub fn h() {}\n",
        "crates/w/src/c.rs": "pub struct Other;\nimpl std::fmt::Display for Other { }\nimpl<R: Copy> Foo for R {}\n",
        "crates/w/examples/probe.rs": "use w::a::f;\nfn main() { f(); }\n",
    }

    def narrow(self, files, scope):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", self.crate(temp, files)):
            return parity.narrow_rust_sources(sorted(files), scope)

    def test_a_closed_scope_drops_the_rest(self):
        kept, note = self.narrow(self.FILES, ["crates/w/src/a.rs", "crates/w/src/b.rs"])
        self.assertIsNone(note)
        self.assertEqual(kept, ["crates/w/examples/probe.rs", "crates/w/src/a.rs", "crates/w/src/b.rs",
                                "crates/w/src/lib.rs#keep:a,b"])

    def test_references_out_of_the_scope_keep_everything(self):
        files = dict(self.FILES, **{"crates/w/src/b.rs": "pub fn h() { crate::c::Other; }\n"})
        kept, note = self.narrow(files, ["crates/w/src/a.rs", "crates/w/src/b.rs"])
        self.assertIn("crate::c::Other", note)
        self.assertEqual(kept, sorted(files))

    def test_impls_elsewhere_for_scope_types_keep_everything(self):
        files = dict(self.FILES, **{"crates/w/src/c.rs": "impl From<i32> for crate::a::Tag { }\n"})
        self.assertIn("Tag", self.narrow(files, ["crates/w/src/a.rs", "crates/w/src/b.rs"])[1])

    def test_new_modules_in_the_crate_root_do_not_count(self):
        keep = {"a", "b"}
        root = self.FILES["crates/w/src/lib.rs"]
        self.assertEqual(parity.crate_root_without_modules(root, keep),
                         parity.crate_root_without_modules(root + "#[cfg(test)]\nmod d;\n", keep))
        self.assertNotEqual(parity.crate_root_without_modules(root, keep),
                            parity.crate_root_without_modules(root + "pub const W: i32 = 2;\n", keep))

    def test_rust_code_blanks_literals_and_comments(self):
        code = parity.rust_code('let a = "{ crate::x }"; // crate::y\nlet c = \'{\'; fn f<\'a>() { r#"}"# }')
        self.assertNotIn("crate", code)
        self.assertEqual(code.count("{"), code.count("}"))
        self.assertIn("<'a>", code)


class ShardTests(unittest.TestCase):
    def test_invalid_shards_are_rejected(self):
        for spec in ("0/8", "9/8", "1/0", "1/-8"):
            with self.subTest(spec=spec), mock.patch.dict(os.environ, {"PARITY_SHARD": spec}):
                with self.assertRaisesRegex(ValueError, "PARITY_SHARD"):
                    parity_shard.shard(list(range(8)))
                with self.assertRaisesRegex(ValueError, "PARITY_SHARD"):
                    parity_shard.label()

    def test_valid_shard_selects_every_nth_case(self):
        with mock.patch.dict(os.environ, {"PARITY_SHARD": "2/3"}):
            self.assertEqual(parity_shard.shard(list(range(7))), [1, 4])
            self.assertEqual(parity_shard.label(), "2-3")

    def test_a_sharded_harness_runs_as_numbered_entries(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            (Path(temp) / "tools").mkdir()
            (Path(temp) / "tools/test_x_parity.py").write_text("PARITY_SHARDS = 3\n")
            (Path(temp) / "tools/test_y_parity.py").write_text("")
            self.assertEqual(sorted(parity.harnesses()), ["x#1", "x#2", "x#3", "y"])
            self.assertEqual(parity.SHARDS["x#2"], "2/3")
            self.assertEqual(parity.base_name("x#2"), "x")

    def test_a_manual_proof_script_is_left_out(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.dict(parity.MANUAL, clear=True):
            (Path(temp) / "tools").mkdir()
            (Path(temp) / "tools/test_x_parity.py").write_text('import sys\nPARITY_MANUAL = "--side java, then --side native"\n')
            (Path(temp) / "tools/test_y_parity.py").write_text("")
            self.assertEqual(sorted(parity.harnesses()), ["y"])
            self.assertEqual(parity.MANUAL, {"x": "--side java, then --side native"})


class OracleArgumentTests(unittest.TestCase):
    def test_an_agent_jar_is_an_input(self):
        from tools import parity_shim
        jar, oracle = Path("/w/seed-agent.jar"), Path("/w/out.jsonl")
        self.assertEqual(parity_shim.named_paths([f"-javaagent:{jar}=a=b", "-Xmx3G", str(oracle)]),
                         [os.path.abspath(jar), os.path.abspath(oracle)])


class ShimDecisionSummaryTests(unittest.TestCase):
    def summary(self, text):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "decisions.jsonl"
            path.write_text(text)
            with mock.patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": ""}), mock.patch("builtins.print") as said:
                parity.report_decisions(path)
        return "\n".join(str(call.args[0]) if call.args else "" for call in said.call_args_list)

    def test_the_summary_counts_reasons_and_seconds(self):
        lines = [{"harness": "a", "reason": "hit", "seconds": 30.0, "call": "c1"},
                 {"harness": "a", "reason": "input-set", "seconds": 50.0, "call": "c2"},
                 {"harness": "b", "reason": "environment", "seconds": 20.0, "variables": ["HORIZON_ORDER"]},
                 {"harness": "b", "reason": "passthrough-pipe", "seconds": None}]
        printed = self.summary("".join(json.dumps(line) + "\n" for line in lines))
        self.assertIn("Java calls: 4 (", printed)
        self.assertIn("Java ran 70 s, replayed 30 s; 1 calls untimed", printed)
        self.assertIn("Most Java seconds not replayed: a 50 s, b 20 s", printed)
        # Nothing from the calls themselves.
        self.assertNotIn("HORIZON_ORDER", printed)
        self.assertNotIn("c1", printed)

    def test_a_broken_line_is_skipped_and_counted(self):
        # A shim that could write only part of its line (a full disk), and one from a newer or older shim.
        printed = self.summary(json.dumps({"harness": "a", "reason": "hit", "seconds": 3}) + "\n" +
                               json.dumps({"harness": "a", "reason": 5}) + "\n" +
                               '{"harness": "a", "reason": "no-rec')
        self.assertIn("Java calls: 1 (hit 1)", printed)
        self.assertIn("2 unreadable lines skipped", printed)

    def test_a_decision_file_that_cannot_be_made_leaves_the_run_without_a_summary(self):
        with tempfile.TemporaryDirectory() as temp:
            # A folder where the file should be.
            with mock.patch.object(parity, "DECISIONS", Path(temp)), mock.patch("builtins.print") as said:
                self.assertIsNone(parity.fresh_decisions())
            self.assertIn("no oracle reuse summary", said.call_args.args[0])
            self.assertNotIn(temp, said.call_args.args[0])  # the repository-relative name only: CI logs are public
            notes = Path(temp) / "decisions.jsonl"
            notes.write_text("an earlier run's\n")
            with mock.patch.object(parity, "DECISIONS", notes):
                self.assertEqual(parity.fresh_decisions(), notes)
            self.assertEqual(notes.read_text(), "")
        with mock.patch("builtins.print") as said:
            parity.report_decisions(None)
        said.assert_not_called()

    def test_no_notes_no_summary(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch("builtins.print") as said:
            parity.report_decisions(Path(temp) / "missing")
            (Path(temp) / "empty").write_text("")
            parity.report_decisions(Path(temp) / "empty")
        said.assert_not_called()


@unittest.skipIf(os.name == "nt", "oracle PATH shimming uses Unix executable symlink semantics")
class OracleReuseTests(unittest.TestCase):
    """tools/parity_shim.py replays a run only when its inputs and outputs are as recorded."""

    def setUp(self):
        import os, subprocess, sys
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.bin, self.shim = root / "bin", root / "shim"
        self.bin.mkdir(); self.shim.mkdir()
        # A fake oracle in a fake JDK (bin/java, release, lib/modules, the VM library): copies its input to its
        # output and counts its runs. It fails on the input "fail", and kills itself on "kill".
        self.release = root / "release"
        self.jdk(root)
        self.fake_java("1")
        (self.shim / "java").symlink_to(Path(parity.__file__).with_name("parity_shim.py"))
        self.input, self.output = root / "work" / "in.txt", root / "work" / "out.txt"
        self.input.parent.mkdir()
        self.input.write_text("a")
        # The shim starts with `#!/usr/bin/env python3`, and a Python's own folder may name it only
        # python3.13 (Homebrew's): a python3 of its own keeps the PATH below enough.
        python = root / "python"
        python.mkdir()
        (python / "python3").symlink_to(sys.executable)
        self.python = sys.executable
        self.decisions = root / "decisions.jsonl"
        self.env = dict(os.environ, PATH=f"{self.shim}:{self.bin}:{python}", PARITY_SHIM_REAL_PATH=str(self.bin),
                        PARITY_SHIM_CACHE=str(root / "cache"), PARITY_SHIM_DECISIONS=str(self.decisions),
                        PARITY_SHIM_HARNESS="fake")
        for name in ("PARITY_JAVA_CACHE", "HORIZON_PHASE", "HORIZON_ORDER", "HORIZON_DATAPACKS", *UNKEYED_VARIABLES):
            self.env.pop(name, None)
        self.subprocess = subprocess

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def jdk(home):
        """The files beside bin/ that make `home` a JDK the shim can fingerprint."""
        (home / "lib" / "server").mkdir(parents=True)
        (home / "release").write_text('JAVA_VERSION="1"\n')
        (home / "lib" / "modules").write_text("modules")
        (home / "lib" / "server" / "libjvm.so").write_text("vm")

    def fake_java(self, version):
        fake = self.bin / "java"
        fake.write_text(f"#!{sys.executable}\nimport sys, pathlib\n"
                        f"if sys.argv[1:] == ['-version']: sys.exit(print('fake java {version}', file=sys.stderr))\n"
                        "runs = pathlib.Path(sys.argv[1]).parent / 'runs'\n"
                        "runs.write_text(str(int(runs.read_text() or 0) + 1) if runs.exists() else '1')\n"
                        "text = pathlib.Path(sys.argv[1]).read_text()\n"
                        "if text == 'fail': sys.exit(2)\n"
                        "if text == 'kill': import os, signal; os.kill(os.getpid(), signal.SIGKILL)\n"
                        "pathlib.Path(sys.argv[2]).write_text(text)\n"
                        "print('done')\n")
        fake.chmod(0o755)

    def call(self, *extra, check=True, stdin=None, **variables):
        return self.subprocess.run(["java", str(self.input), str(self.output), *extra],
                                   env=dict(self.env, **variables), capture_output=True, text=True,
                                   stdin=self.subprocess.DEVNULL if stdin is None else stdin, check=check)

    def oracle(self, *extra, **variables):
        self.assertEqual(self.call(*extra, **variables).stdout, "done\n")
        return self.runs()

    def runs(self):
        return int((self.input.parent / "runs").read_text())

    def reasons(self):
        """The decision lines since the last call, then forgets them."""
        text = self.decisions.read_text() if self.decisions.exists() else ""
        lines = [json.loads(line) for line in text.splitlines()]
        self.decisions.unlink(missing_ok=True)
        for line in lines:
            self.assertEqual((line["harness"], line["program"]), ("fake", "java"))
        return [line["reason"] for line in lines]

    def last(self):
        return json.loads(self.decisions.read_text().splitlines()[-1])

    def test_each_call_says_why_it_was_replayed_or_not(self):
        extra = self.input.parent / "extra"
        extra.mkdir()
        self.oracle(extra)
        self.assertEqual(self.reasons(), ["no-record"])
        self.oracle(extra)
        line = self.last()
        self.assertEqual(self.reasons(), ["hit"])
        self.assertIsNotNone(line["seconds"])
        # Only an opaque id of the call, and nothing else from it.
        self.assertEqual(set(line), {"harness", "program", "reason", "seconds", "call"})
        self.assertEqual(len(line["call"]), 12)
        self.input.write_text("b")
        self.oracle(extra)
        self.assertEqual(self.reasons(), ["input-content"])
        (extra / "new.txt").write_text("x")
        self.oracle(extra)
        line = self.last()
        self.assertEqual(self.reasons(), ["input-set"])
        self.assertEqual((line["added"], line["removed"]), (1, 0))
        self.output.write_text("tampered")
        self.oracle(extra)
        self.assertEqual(self.reasons(), ["output-stamp"])
        self.oracle(extra, HORIZON_PHASE="terrain")
        line = self.last()
        self.assertEqual(self.reasons(), ["environment"])
        self.assertEqual(line["variables"], ["HORIZON_PHASE"])
        self.fake_java("2.0")  # another size, as a JDK update gives
        self.oracle(extra, HORIZON_PHASE="terrain")
        self.assertEqual(self.reasons(), ["java-runtime"])
        self.call("-version")
        self.call(extra, stdin=self.subprocess.PIPE, HORIZON_PHASE="terrain")
        self.call(extra, PARITY_JAVA_CACHE="0", HORIZON_PHASE="terrain")
        self.call(extra, HORIZON_PHASE="terrain", JAVA_TOOL_OPTIONS="-javaagent:/x/agent.jar")
        self.input.write_text("fail")
        self.assertEqual(self.call(extra, check=False, HORIZON_PHASE="terrain").returncode, 2)
        line = self.last()
        self.assertEqual(self.reasons(), ["passthrough-version", "passthrough-pipe", "passthrough-off",
                                          "passthrough-jvm-options", "passthrough-failed"])
        self.assertEqual(line["miss"], "output-stamp")  # the passthroughs above wrote it

    def test_the_call_id_says_which_calls_are_the_same_within_a_run_only(self):
        self.oracle(PARITY_SHIM_RUN="one")
        self.oracle(PARITY_SHIM_RUN="one")
        first, second = [json.loads(line)["call"] for line in self.decisions.read_text().splitlines()]
        self.assertEqual(first, second)
        self.reasons()
        self.oracle(PARITY_SHIM_RUN="two")
        self.assertNotEqual(self.last()["call"], first)

    def test_a_java_killed_by_a_signal_gives_the_same_status_keyed_or_not(self):
        # 256 - 9, as the shim's run of a recorded call has always given; the new passthroughs keep it.
        self.input.write_text("kill")
        self.assertEqual(self.call(check=False).returncode, 247)
        self.assertEqual(self.call(check=False, JAVA_TOOL_OPTIONS="-Dx=1").returncode, 247)
        self.assertEqual(self.reasons(), ["passthrough-failed", "passthrough-jvm-options"])

    def test_a_piped_call_with_jvm_options_is_passed_through_as_on_main(self):
        self.call(stdin=self.subprocess.PIPE, JAVA_TOOL_OPTIONS="-Dx=1")
        self.assertEqual(self.reasons(), ["passthrough-pipe"])

    def test_jvm_options_from_the_environment_are_never_reused(self):
        for variable in UNKEYED_VARIABLES:
            with self.subTest(variable=variable):
                before = self.oracle(**{variable: "-Dx=1"})
                self.assertEqual(self.oracle(**{variable: "-Dx=1"}), before + 1)
                self.assertEqual(self.reasons(), ["passthrough-jvm-options"] * 2)
        # Set but empty, as a CI step may leave one: nothing to establish.
        self.oracle(CLASSPATH="")
        self.assertEqual(self.reasons(), ["no-record"])

    def test_the_key_holds_the_runtime_behind_the_launcher(self):
        self.assertEqual(self.oracle(), 1)
        # The JDK updated in place: the launcher is the same file, and prints the same version.
        self.release.write_text('JAVA_VERSION="1"\nIMPLEMENTOR="another"\n')
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.reasons(), ["no-record", "java-runtime", "hit"])
        # The VM library replaced: the same launcher, release file and class library.
        (self.bin.parent / "lib" / "server" / "libjvm.so").write_text("another vm")
        self.assertEqual(self.oracle(), 3)
        self.assertEqual(self.reasons(), ["java-runtime"])

    def test_a_runtime_link_retargeted_to_an_equal_looking_file_changes_the_key(self):
        server = self.bin.parent / "lib" / "server"
        for name, text in (("a.so", "vm"), ("b.so", "VM")):
            (server / name).write_text(text)
            os.utime(server / name, ns=(10**18, 10**18))
        (server / "libjvm.so").unlink()
        (server / "libjvm.so").symlink_to(server / "a.so")
        self.assertEqual(self.oracle(), 1)
        (server / "libjvm.so").unlink()
        (server / "libjvm.so").symlink_to(server / "b.so")  # the same size and mtime, another file
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.reasons(), ["no-record", "java-runtime"])

    def test_a_vm_library_byte_rewritten_under_its_old_size_and_mtime_changes_the_key(self):
        # The memo of its hash is not consulted: the rewrite changed its ctime.
        vm = self.bin.parent / "lib" / "server" / "libjvm.so"
        self.assertEqual(self.oracle(), 1)
        before = vm.stat()
        vm.write_text("vM")
        os.utime(vm, ns=(before.st_atime_ns, before.st_mtime_ns))
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.reasons(), ["no-record", "java-runtime"])

    def test_the_same_jdk_unpacked_again_replays(self):
        # As on a fresh runner: the same bytes at the same path, but new files (inodes, ctimes) and other mtimes.
        home = self.bin.parent
        self.assertEqual(self.oracle(), 1)
        for path in (self.bin / "java", self.release, home / "lib" / "modules", home / "lib" / "server" / "libjvm.so"):
            data, mode = path.read_bytes(), path.stat().st_mode
            path.unlink()
            path.write_bytes(data)
            path.chmod(mode)
            os.utime(path, ns=(10**18, 10**18))
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.reasons(), ["no-record", "hit"])

    def in_process(self):
        """runtime_digest called here, with the cache and the memo in this test's folder."""
        from tools import parity_shim
        root = self.bin.parent
        patches = [mock.patch.object(parity_shim, "CACHE", root / "cache"),
                   mock.patch.object(parity_shim, "RUNTIME_MEMO", root / "runtime-memo")]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        return parity_shim, root / "runtime-memo"

    def test_two_jdk_homes_with_the_same_content_give_the_same_runtime(self):
        shim, _ = self.in_process()
        home, other = self.bin.parent, self.bin.parent / "elsewhere"
        for name in ("bin/java", "release", "lib/modules", "lib/server/libjvm.so"):
            (other / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(home / name, other / name)  # new inodes and mtimes
        first = shim.runtime_digest(str(self.bin / "java"))
        self.assertEqual(shim.runtime_digest(str(other / "bin" / "java")), first)
        (other / "lib" / "server" / "libjvm.so").write_text("vM")
        self.assertNotEqual(shim.runtime_digest(str(other / "bin" / "java")), first)

    def test_a_jdk_file_changed_while_it_is_hashed_or_probed_is_not_keyed(self):
        shim, memo = self.in_process()
        vm = self.bin.parent / "lib" / "server" / "libjvm.so"
        hash_file, run = shim.bytes_digest, shim.subprocess.run

        def hashed_then_changed(path):
            result = hash_file(path)
            if os.path.realpath(path) == os.path.realpath(vm):
                # Another size: a rewrite of equal size within one timestamp tick of the first stat shows in no field.
                vm.write_text("the vm replaced between the stats")
            return result

        def probed_then_changed(*arguments, **options):
            result = run(*arguments, **options)
            vm.write_text("the vm replaced while the launcher was probed")
            return result

        with mock.patch.object(shim, "bytes_digest", hashed_then_changed), self.assertRaises(shim.Unkeyed):
            shim.runtime_digest(str(self.bin / "java"))
        self.assertEqual(list(memo.glob("*.json")), [])
        with mock.patch.object(shim.subprocess, "run", probed_then_changed), self.assertRaises(shim.Unkeyed):
            shim.runtime_digest(str(self.bin / "java"))
        self.assertEqual(list(memo.glob("*.json")), [])
        shim.runtime_digest(str(self.bin / "java"))  # nothing changes meanwhile
        self.assertEqual(len(list(memo.glob("*.json"))), 1)

    def test_a_call_whose_runtime_changes_while_it_is_probed_runs_unrecorded(self):
        root = self.bin.parent
        vm, java = root / "lib" / "server" / "libjvm.so", self.bin / "java"
        probe = "if sys.argv[1:] == ['-version']:"
        java.write_text(java.read_text().replace(probe, f"{probe} open({str(vm)!r}, 'a').write('x')\n{probe}"))
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.reasons(), ["passthrough-runtime"] * 2)
        self.assertEqual(list((root / "cache").glob("*.json")) + list((root / "runtime-memo").glob("*")), [])

    def test_a_machine_id_that_is_there_but_unreadable_leaves_the_memo_unused(self):
        shim, memo = self.in_process()
        machine_id = self.bin.parent / "machine-id"
        machine_id.write_bytes(bytes([0xFF, 0xFE]))  # not text
        with mock.patch.object(shim, "MACHINE_ID", str(machine_id)):
            self.assertIsNone(shim.machine())
            first = shim.runtime_digest(str(self.bin / "java"))
            self.assertEqual(list(memo.glob("*.json")), [])
            machine_id.unlink()
            machine_id.mkdir()  # can't be read as a file
            self.assertIsNone(shim.machine())
            machine_id.rmdir()  # missing, as on macOS: empty, and the memo is used
            self.assertIsNotNone(shim.machine())
            self.assertEqual(shim.runtime_digest(str(self.bin / "java")), first)
            self.assertEqual(len(list(memo.glob("*.json"))), 1)

    def test_another_machines_memo_entry_is_ignored(self):
        shim, memo = self.in_process()
        first = shim.runtime_digest(str(self.bin / "java"))
        [entry] = memo.glob("*.json")
        stored = json.loads(entry.read_text())
        # Restored from elsewhere with matching stat data, and wrong content: never trusted.
        entry.write_text(json.dumps(dict(stored, machine="elsewhere", content=["wrong"] * 5)))
        self.assertEqual(shim.runtime_digest(str(self.bin / "java")), first)
        self.assertEqual(json.loads(entry.read_text()), stored)

    def test_the_memo_is_beside_the_cache_not_in_it(self):
        self.oracle()
        root = self.bin.parent
        self.assertEqual(len(list((root / "runtime-memo").glob("*.json"))), 1)
        self.assertEqual([path.name for path in (root / "cache").iterdir() if "runtime" in path.name], [])

    def test_javacs_ct_sym_is_part_of_the_key_present_or_not(self):
        ct_sym = self.bin.parent / "lib" / "ct.sym"
        self.assertEqual(self.oracle(), 1)
        ct_sym.write_text("release 21")
        self.assertEqual(self.oracle(), 2)
        ct_sym.write_text("release 22")
        self.assertEqual(self.oracle(), 3)
        ct_sym.unlink()
        # Absent again: the first record's key, whose output the runs since have rewritten.
        self.assertEqual(self.oracle(), 4)
        self.assertEqual(self.reasons(), ["no-record", "java-runtime", "java-runtime", "output-stamp"])

    def test_a_launcher_outside_a_whole_jdk_is_never_reused(self):
        home = self.bin.parent
        modules = home / "lib" / "modules"
        # /usr/bin/java beside a /usr/lib/modules folder: not a JDK's class library.
        modules.unlink()
        modules.mkdir()
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 2)
        modules.rmdir()
        modules.write_text("modules")
        (home / "lib" / "server" / "libjvm.so").unlink()  # no VM library
        self.assertEqual(self.oracle(), 3)
        (home / "lib" / "server" / "libjvm.so").write_text("vm")
        self.release.unlink()
        self.assertEqual(self.oracle(), 4)
        self.assertEqual(self.reasons(), ["passthrough-runtime"] * 4)
        # A launcher whose folder is not bin/ (a stub, a selector) neither, JDK files beside it or not.
        self.release.write_text('JAVA_VERSION="1"\n')
        stub = home / "stub"
        stub.mkdir()
        (stub / "java").symlink_to(self.bin / "java")
        self.assertEqual(self.oracle(), 5)  # through the link: the real launcher is in bin/
        (stub / "java").unlink()
        (stub / "java").write_bytes((self.bin / "java").read_bytes())
        (stub / "java").chmod(0o755)
        self.env["PARITY_SHIM_REAL_PATH"] = str(stub)
        self.assertEqual(self.oracle(), 6)
        self.assertEqual(self.oracle(), 7)
        self.assertEqual(self.reasons(), ["no-record", "passthrough-runtime", "passthrough-runtime"])

    def test_the_key_holds_the_path_java_searches_not_the_harness_one(self):
        self.assertEqual(self.oracle(), 1)
        # The harness's PATH is the shim's folder and the real PATH: another order or folder after it is the same
        # real PATH...
        self.assertEqual(self.oracle(PATH=f"{self.shim}:{self.env['PATH']}:/nowhere"), 1)
        # ...but another real PATH is not.
        self.assertEqual(self.oracle(PARITY_SHIM_REAL_PATH=f"{self.bin}:/nowhere"), 2)
        self.assertEqual(self.reasons(), ["no-record", "hit", "environment"])

    def test_the_key_holds_the_variables_oracles_read_and_what_they_name(self):
        order = self.input.parent.parent / "order.txt"
        order.write_text("0 0\n")
        self.assertEqual(self.oracle(HORIZON_PHASE="full", HORIZON_ORDER=str(order)), 1)
        # Variables that change between runs of the same call stay out of it.
        self.assertEqual(self.oracle(HORIZON_PHASE="full", HORIZON_ORDER=str(order), TERM="dumb", PARITY_RUN_ID="7",
                                     GITHUB_RUN_ID="8", HORIZON_HOME="/elsewhere", BASE_SHA="c0ffee", GH_TOKEN="t",
                                     XDG_SESSION_ID="3"), 1)
        self.assertEqual(self.reasons(), ["no-record", "hit"])
        # The same file name with other content: Horizon writes HORIZON_ORDER's file.
        order.write_text("1 1\n")
        self.assertEqual(self.oracle(HORIZON_PHASE="full", HORIZON_ORDER=str(order)), 2)
        self.assertEqual(self.oracle(HORIZON_PHASE="terrain", HORIZON_ORDER=str(order)), 3)
        self.assertEqual(self.oracle(HORIZON_PHASE="terrain", HORIZON_ORDER=str(order), SOMETHING_NEW="1"), 4)
        self.assertEqual(self.oracle(HORIZON_PHASE="terrain", HORIZON_ORDER=str(order), SOMETHING_NEW="1"), 4)
        self.assertEqual(self.reasons(), ["environment", "environment", "environment", "hit"])

    def test_a_call_on_another_hosted_runner_replays(self):
        # A Blacksmith runner sets these per VM (its name and id, its Turborepo cache's address and token, and two
        # undocumented ones): two runners' calls that differ only in them are the same call.
        first = dict(PETNAME="calm-otter", VM_ID="vm-1", TURBO_API="http://10.0.0.1:1", TURBO_TOKEN="a",
                     GIT_MIRROR_PROXY_URL="http://10.0.0.1:3", METADATA_PORT="1")
        second = dict(PETNAME="brisk-heron", VM_ID="vm-2", TURBO_API="http://10.0.0.2:2", TURBO_TOKEN="b",
                      GIT_MIRROR_PROXY_URL="http://10.0.0.2:4", METADATA_PORT="2")
        self.assertEqual(self.oracle(**first), 1)
        self.assertEqual(self.oracle(**second), 1)
        # Only those names: a variable that merely starts like one is still in the key.
        self.assertEqual(self.oracle(**second, TURBO_TEAM="x"), 2)
        self.assertEqual(self.last()["variables"], ["TURBO_TEAM"])
        self.assertEqual(self.reasons(), ["no-record", "hit", "environment"])

    def test_a_record_cut_short_is_a_miss_and_not_a_failure(self):
        # As a cache entry restored in part leaves it: the call runs again, records anew, then replays.
        cache = Path(self.env["PARITY_SHIM_CACHE"])
        self.assertEqual(self.oracle(), 1)
        record = next(cache.glob("*.json"))
        damages = [lambda: record.with_suffix(".stdout").unlink(), lambda: record.with_suffix(".stderr").unlink(),
                   lambda: record.write_text(record.read_text()[:20]), lambda: record.write_text('{"inputs": []}')]
        for runs, damage in enumerate(damages, 2):
            damage()
            self.assertEqual(self.oracle(), runs)
        self.assertEqual(self.oracle(), 5)
        self.assertEqual(self.reasons(), ["no-record", *["incomplete-record"] * 4, "hit"])

    def test_a_stream_that_is_not_the_records_is_a_miss(self):
        # The record stamps each stream's length and SHA-256: one cut short, swapped, or of another write of the same
        # call is a miss (and is recorded anew), never replayed. A record from before the stamps is a miss too.
        cache = Path(self.env["PARITY_SHIM_CACHE"])
        self.assertEqual(self.oracle(), 1)
        record = next(cache.glob("*.json"))
        stdout, stderr = record.with_suffix(".stdout"), record.with_suffix(".stderr")
        self.assertEqual(json.loads(record.read_text())["streams"]["stdout"],
                         [5, hashlib.sha256(b"done\n").hexdigest()])

        def swap():
            first, second = stdout.read_bytes(), stderr.read_bytes()
            stdout.write_bytes(second)
            stderr.write_bytes(first)

        def unstamped():
            saved = json.loads(record.read_text())
            del saved["streams"]
            record.write_text(json.dumps(saved))

        damages = [lambda: stdout.write_bytes(b"do"), swap, lambda: stdout.write_bytes(b"dXne\n"),
                   lambda: stderr.write_bytes(b"x"), unstamped]
        for runs, damage in enumerate(damages, 2):
            damage()
            self.assertEqual(self.oracle(), runs)
        self.assertEqual(self.oracle(), 6)
        self.assertEqual(self.reasons(), ["no-record", *["incomplete-record"] * 5, "hit"])

    def test_a_record_is_written_after_its_streams_and_a_crash_between_never_replays_wrong_output(self):
        from tools import parity_shim

        class Crash(Exception):
            pass

        def state(key, record):
            saved = parity_shim.recorded(record)
            return None if saved is None else parity_shim.replayable(key, saved)

        def document(streams):
            return {"inputs": {}, "outputs": {}, "streams": {name: parity_shim.stream_stamp(data)
                                                             for name, data in zip(parity_shim.STREAMS, streams)}}

        old, new = [b"old out\n", b"old err\n"], [b"new out\n", b"new err\n"]
        # After how many renames the write stops: 0 (none), 1 (stdout), 2 (both streams), 3 (all, no crash).
        for old_streams, renames, expected in [(old, 0, old), (old, 1, None), (old, 2, None), (old, 3, new),
                                               (None, 0, None), (None, 1, None), (None, 2, None), (None, 3, new)]:
            with self.subTest(old=bool(old_streams), renames=renames), tempfile.TemporaryDirectory() as temp, \
                    mock.patch.object(parity_shim, "CACHE", Path(temp)):
                key, record = "k", Path(temp) / "k.json"
                if old_streams:
                    parity_shim.publish(key, record, old_streams, document(old_streams))
                replace, done = Path.replace, []

                def stopping(path, target):
                    if len(done) == renames:
                        raise Crash()
                    done.append(target)
                    return replace(path, target)

                with mock.patch.object(Path, "replace", stopping):
                    try:
                        parity_shim.publish(key, record, new, document(new))
                    except Crash:
                        pass
                # Streams first, the record last; no temporary file is left.
                self.assertEqual([path.name for path in done], ["k.stdout", "k.stderr", "k.json"][:renames])
                self.assertEqual(sorted(path.name for path in Path(temp).iterdir() if path.suffix == ".tmp"), [])
                self.assertEqual(state(key, record), expected)

    def test_a_hash_database_that_fails_is_left_alone_and_files_are_hashed_directly(self):
        # Another shim may still have it open: a shim never removes or remakes it, and from its first failure on
        # hashes every file itself, a miss and never a failure.
        import sqlite3
        from tools import parity_shim
        expected = hashlib.sha256(Path(parity_shim.__file__).read_bytes()).hexdigest()

        class Failing:
            """A connection whose every statement meets a damaged page."""
            def execute(self, *args):
                raise sqlite3.DatabaseError("database disk image is malformed")

            def commit(self):
                raise sqlite3.DatabaseError("database disk image is malformed")

            def close(self):
                pass

        class FailingStore(Failing):
            def execute(self, statement, *args):
                if statement.startswith("select"):
                    return sqlite3.connect(":memory:").execute("select 1 where 0")
                return super().execute(statement, *args)

        class Busy(Failing):
            def execute(self, *args):
                raise sqlite3.OperationalError("database is locked")

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"):
            path = parity_shim.CACHE / "hashes.sqlite"
            hashes = parity_shim.Hashes()
            hashes.db.execute("insert into hashes values ('marker', 0, 0, 'x')")
            hashes.db.commit()
            hashes.close()
            for failing in (Failing, FailingStore, Busy):
                with self.subTest(failing.__name__):
                    hashes = parity_shim.Hashes()
                    hashes.db.close()
                    hashes.db = failing()
                    self.assertEqual(hashes.file(parity_shim.__file__), expected)
                    self.assertIsNone(hashes.db)
                    self.assertEqual(hashes.file(parity_shim.__file__), expected)
                    # The database is as it was: nothing removed or made again.
                    db = sqlite3.connect(path)
                    self.assertEqual(db.execute("select sha from hashes where path = 'marker'").fetchone(), ("x",))
                    db.close()
            # A file that is no database at all, as a restore cut short leaves it: left as it is.
            for name in ("hashes.sqlite-wal", "hashes.sqlite-shm"):
                (parity_shim.CACHE / name).unlink(missing_ok=True)
            garbage = b"not a database" * 512
            path.write_bytes(garbage)
            hashes = parity_shim.Hashes()
            self.assertIsNone(hashes.db)
            self.assertEqual(hashes.file(parity_shim.__file__), expected)
            self.assertEqual(path.read_bytes(), garbage)

    def test_a_damaged_hash_database_is_removed_before_the_shims_start_only_by_a_run_owning_the_cache(self):
        import sqlite3
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"):
            cache = parity_shim.CACHE
            path = cache / "hashes.sqlite"
            parity.check_oracle_hashes(cache, repair=True)  # none yet: nothing to do
            self.assertFalse(path.exists())
            hashes = parity_shim.Hashes()
            hashes.db.execute("insert into hashes values ('marker', 0, 0, 'x')")
            hashes.db.commit()
            hashes.close()
            # A sound one is kept.
            parity.check_oracle_hashes(cache, repair=True)
            db = sqlite3.connect(path)
            self.assertEqual(db.execute("select sha from hashes where path = 'marker'").fetchone(), ("x",))
            # One locked by another process, or not checked in time, is left as it is.
            db.execute("begin exclusive")
            parity.check_oracle_hashes(cache, repair=True)
            db.execute("rollback")
            db.close()
            parity.check_oracle_hashes(cache, repair=True, seconds=-1)
            self.assertTrue(path.exists())
            # One that is no database: without repair (another parity run in the checkout may have it open) only
            # named, every file kept; with repair removed with its WAL files, and the first shim makes it again.
            path.write_bytes(b"not a database" * 512)
            (cache / "hashes.sqlite-wal").write_bytes(b"x")
            (cache / "hashes.sqlite-shm").write_bytes(b"x")
            with mock.patch("builtins.print") as said:
                parity.check_oracle_hashes(cache)
            self.assertIn("is damaged: left as it is", said.call_args.args[0])
            # (SQLite itself may drop an invalid -wal on closing its connection, only when no other has it open.)
            self.assertEqual(path.read_bytes(), b"not a database" * 512)
            for name in ("hashes.sqlite-wal", "hashes.sqlite-shm"):
                (cache / name).write_bytes(b"x")
            parity.check_oracle_hashes(cache, repair=True)
            self.assertEqual(sorted(p.name for p in cache.iterdir() if p.name.startswith("hashes.sqlite")), [])
            hashes = parity_shim.Hashes()
            self.assertTrue(parity_shim.in_wal_mode(path))
            hashes.close()
            # One with a damaged page: a page of the table overwritten.
            db = sqlite3.connect(path)
            db.execute("pragma journal_mode=delete")
            db.executemany("insert into hashes values (?, 0, 0, 'x')", ((f"p{i:06d}",) for i in range(5000)))
            db.commit()
            db.close()
            data = bytearray(path.read_bytes())
            data[4096 * 3:4096 * 3 + 512] = b"\xff" * 512
            path.write_bytes(bytes(data))
            parity.check_oracle_hashes(cache)
            self.assertTrue(path.exists())
            parity.check_oracle_hashes(cache, repair=True)
            self.assertFalse(path.exists())
        # Repair only when the caller says so: tools/parity.py's run takes it from PARITY_REPAIR_ORACLE_HASHES=1, which
        # only parity-hosted.yml sets (tests/test_ci_runners.py).
        self.assertIn('check_oracle_hashes(STATE_DIR / "oracle-cache", '
                      'os.environ.get("PARITY_REPAIR_ORACLE_HASHES") == "1")',
                      Path(parity.__file__).read_text(encoding="utf-8"))

    def test_a_damaged_hash_database_never_fails_the_call(self):
        cache = Path(self.env["PARITY_SHIM_CACHE"])
        cache.mkdir()
        (cache / "hashes.sqlite").write_bytes(b"\x00garbage" * 1024)
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.reasons(), ["no-record", "hit"])
        # The shims left it as it was: tools/parity.py removes it before the next run's shims start.
        self.assertEqual((cache / "hashes.sqlite").read_bytes(), b"\x00garbage" * 1024)

    def test_an_environment_miss_names_every_variable_that_differs(self):
        # Against the last run of the same command line: one changed, one added, one gone, and an ignored one that
        # changed too, which is not named. Names only: no value reaches the line.
        self.assertEqual(self.oracle(HORIZON_PHASE="full", HORIZON_A="1", HORIZON_GONE="x", VM_ID="vm-1"), 1)
        self.assertEqual(self.oracle(HORIZON_PHASE="terrain", HORIZON_A="1", HORIZON_NEW="y", VM_ID="vm-2"), 2)
        line = self.last()
        self.assertEqual(line["reason"], "environment")
        self.assertEqual(line["variables"], ["HORIZON_GONE", "HORIZON_NEW", "HORIZON_PHASE"])
        self.assertNotIn("terrain", json.dumps(line))

    def test_a_datapack_folder_is_keyed_through_its_linked_folders(self):
        root = self.input.parent.parent
        packs, elsewhere = root / "datapacks", root / "elsewhere" / "pack"
        packs.mkdir()
        elsewhere.mkdir(parents=True)
        (elsewhere / "pack.mcmeta").write_text("1")
        (packs / "linked").symlink_to(elsewhere, target_is_directory=True)
        (elsewhere / "loop").symlink_to(packs, target_is_directory=True)  # a cycle: named once, not walked forever
        self.assertEqual(self.oracle(HORIZON_DATAPACKS=str(packs)), 1)
        self.assertEqual(self.oracle(HORIZON_DATAPACKS=str(packs)), 1)
        (elsewhere / "pack.mcmeta").write_text("2")
        self.assertEqual(self.oracle(HORIZON_DATAPACKS=str(packs)), 2)
        self.assertEqual(self.reasons(), ["no-record", "hit", "environment"])

    def test_a_datapack_link_retargeted_to_an_equal_looking_pack_changes_the_key(self):
        root = self.input.parent.parent
        packs, first, second = root / "datapacks", root / "a" / "pack", root / "b" / "pack"
        for folder, content in ((first, "1"), (second, "2")):
            folder.mkdir(parents=True)
            (folder / "pack.mcmeta").write_text(content)
            os.utime(folder / "pack.mcmeta", ns=(10**18, 10**18))  # the same size and mtime in both
        packs.mkdir()
        (packs / "linked").symlink_to(first, target_is_directory=True)
        self.assertEqual(self.oracle(HORIZON_DATAPACKS=str(packs)), 1)
        (packs / "linked").unlink()
        (packs / "linked").symlink_to(second, target_is_directory=True)
        self.assertEqual(self.oracle(HORIZON_DATAPACKS=str(packs)), 2)
        # The same bytes as before, size and mtime kept: hashed again, not taken from a memo.
        (second / "pack.mcmeta").write_text("3")
        os.utime(second / "pack.mcmeta", ns=(10**18, 10**18))
        self.assertEqual(self.oracle(HORIZON_DATAPACKS=str(packs)), 3)
        self.assertEqual(self.reasons(), ["no-record", "environment", "environment"])

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0, "root reads any folder")
    def test_an_unreadable_datapack_folder_is_passed_through_without_a_traceback(self):
        root = self.input.parent.parent
        packs = root / "datapacks"
        (packs / "locked").mkdir(parents=True)
        (packs / "locked").chmod(0)
        try:
            result = self.call(HORIZON_DATAPACKS=str(packs))
        finally:
            (packs / "locked").chmod(0o755)
        self.assertEqual(result.stdout, "done\n")
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(self.reasons(), ["passthrough-unreadable"])

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0, "root searches any folder")
    def test_an_order_file_behind_an_unsearchable_folder_is_not_keyed_as_missing(self):
        locked = self.input.parent.parent / "locked"
        locked.mkdir()
        (locked / "order.txt").write_text("a\n")
        locked.chmod(0)
        try:
            result = self.call(HORIZON_ORDER=str(locked / "order.txt"))
        finally:
            locked.chmod(0o755)
        self.assertEqual(result.stdout, "done\n")
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(self.reasons(), ["passthrough-unreadable"])

    def test_notes_that_cannot_be_written_change_nothing(self):
        # A folder where the file should be: every open fails.
        self.env["PARITY_SHIM_DECISIONS"] = str(self.input.parent)
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)  # replayed, stdout included
        self.assertEqual(self.call(PARITY_JAVA_CACHE="0").stdout, "done\n")  # the passthrough still runs Java
        self.assertEqual(self.runs(), 2)
        self.input.write_text("fail")
        self.assertEqual(self.call(check=False).returncode, 2)  # Java's own status

    def test_an_unwritable_call_index_changes_nothing(self):
        # A file where the calls/ folder should be: the index, only for saying why a later call missed, can't be made.
        cache = Path(self.env["PARITY_SHIM_CACHE"])
        cache.mkdir()
        (cache / "calls").write_text("")
        result = self.call()
        self.assertEqual((result.returncode, result.stdout), (0, "done\n"))
        self.assertEqual(self.oracle(), 1)  # recorded all the same, and replayed
        self.assertEqual(self.reasons(), ["no-record", "hit"])

    def test_parallel_shims_append_whole_lines(self):
        child = ("import os, sys\nsys.path.insert(0, sys.argv[1])\nimport parity_shim\n"
                 "for index in range(200):\n"
                 "    parity_shim.decide('java', 'hit', 1.0, 'k' * 64, filler='x' * 20000, index=index)\n")
        tools = str(Path(parity.__file__).parent)
        env = dict(self.env, PARITY_SHIM_HARNESS="fake")
        children = [self.subprocess.Popen([sys.executable, "-c", child, tools], env=env) for _ in range(8)]
        for process in children:
            self.assertEqual(process.wait(timeout=120), 0)
        lines = self.decisions.read_text().splitlines()
        self.assertEqual(len(lines), 1600)
        self.assertEqual(sorted(json.loads(line)["index"] for line in lines), sorted(list(range(200)) * 8))

    def test_reuses_until_an_input_or_output_changes(self):
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)  # replayed, stdout included
        self.input.write_text("a")  # same content, new mtime: still reusable
        self.assertEqual(self.oracle(), 1)
        self.input.write_text("b")
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.output.read_text(), "b")
        self.output.write_text("tampered")
        self.assertEqual(self.oracle(), 3)
        self.assertEqual(self.output.read_text(), "b")

    def wrapper(self, folder, put_the_shim_back=False):
        """A `java` that runs the first other `java` on PATH, as JDK selectors and build wrappers do. It stops
        itself after a few rounds, so a shim that loops fails this test instead of filling the machine."""
        folder.mkdir(parents=True)
        path = f"os.environ['PATH'] = {str(self.shim)!r} + os.pathsep + os.environ['PATH']\n" if put_the_shim_back else ""
        (folder / "java").write_text(
            f"#!{self.python}\nimport os, sys, shutil\nfrom pathlib import Path\n"
            "rounds = int(os.environ.get('WRAPPER_ROUNDS') or 0)\n"
            "if rounds > 6: sys.exit(97)\n"
            "os.environ['WRAPPER_ROUNDS'] = str(rounds + 1)\n" + path +
            "me = Path(__file__).resolve()\n"
            "other = next(c for d in os.environ['PATH'].split(os.pathsep) if (c := shutil.which('java', path=d))"
            " and Path(c).resolve() != me)\n"
            "os.execv(other, [other, *sys.argv[1:]])\n")
        (folder / "java").chmod(0o755)

    def test_a_wrapper_that_looks_java_up_on_path_reaches_the_real_one(self):
        # Outside a JDK, the wrapper is never keyed (passthrough-runtime): each call reaches the real one.
        wrapper = self.bin.parent / "wrapper"
        self.wrapper(wrapper)
        self.env["PATH"] = f"{self.shim}:{wrapper}:{self.bin}:{self.bin.parent / 'python'}"
        self.env["PARITY_SHIM_REAL_PATH"] = f"{wrapper}:{self.bin}"
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.reasons(), ["passthrough-runtime"] * 2)

    def test_a_wrapper_that_puts_the_shim_back_stops_with_a_reason(self):
        wrapper = self.bin.parent / "wrapper"
        self.wrapper(wrapper, put_the_shim_back=True)
        # The shim's `env python3` must find a python3 on the real PATH.
        self.env["PARITY_SHIM_REAL_PATH"] = f"{wrapper}:{self.bin}:{self.bin.parent / 'python'}"
        result = self.subprocess.run(["java", str(self.input), str(self.output)], env=self.env, capture_output=True,
                                     text=True, stdin=self.subprocess.DEVNULL, timeout=60)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("started inside itself", result.stderr)

    def test_a_real_path_that_holds_the_shim_skips_it(self):
        from tools import parity_shim
        with mock.patch.dict(os.environ, PARITY_SHIM_REAL_PATH=f"{self.shim}:{self.bin}"):
            self.assertEqual(Path(parity_shim.real("java")), self.bin / "java")


# Each child imports the shim, says it is ready, waits for the go file, and only then opens the cache: all of them
# open the empty cache together, as shims do at a cold start.
RACE_CHILD = """import os, sys
sys.path.insert(0, sys.argv[1])
import parity_shim
open(sys.argv[3], "w").close()
while not os.path.exists(sys.argv[2]):
    pass
parity_shim.Hashes().file(parity_shim.__file__)
"""

# What a shim from before hashes.lock runs, ignoring the lock.
OLD_SHIM = ("pragma journal_mode=wal",
            "create table if not exists hashes (path text primary key, size int, mtime int, sha text)")


def shim_race(tools, shims=16, rounds=8, seconds=60):
    """[round, exit code, last stderr line] for each shim that failed, in `rounds` rounds of `shims` shims released
    at once on an empty cache; each round has one deadline."""
    failures = []
    for round in range(rounds):
        with tempfile.TemporaryDirectory() as temp:
            go, ready = Path(temp) / "go", [Path(temp) / f"ready{index}" for index in range(shims)]
            env = dict(os.environ, PARITY_SHIM_CACHE=str(Path(temp) / "oracle-cache"))
            deadline = time.monotonic() + seconds
            children = []
            try:
                for flag in ready:
                    children.append(subprocess.Popen([sys.executable, "-c", RACE_CHILD, str(tools), str(go), str(flag)],
                                                     env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                                     text=True))
                while not all(flag.exists() for flag in ready):
                    if time.monotonic() > deadline or any(child.poll() is not None for child in children):
                        break
                    time.sleep(0.005)
                go.touch()
                for child in children:
                    error = child.communicate(timeout=max(0.1, deadline - time.monotonic()))[1]
                    if child.returncode:
                        failures.append([round, child.returncode, (error.strip().splitlines() or [""])[-1]])
            finally:
                for child in children:
                    if child.poll() is None:
                        child.kill()
                    child.communicate()
    return failures


class OracleCacheCreationTests(unittest.TestCase):
    """Parallel harnesses start their oracles at once: on an empty cache every shim opens the hash database together,
    and none may fail ("database is locked" when they all switched it to WAL, 2026-11-20)."""

    def test_shims_starting_at_once_on_an_empty_cache_all_get_it(self):
        self.assertEqual(shim_race(Path(parity.__file__).parent), [])

    def test_a_shim_from_before_the_lock_creating_it_meanwhile_keeps_its_writes(self):
        import sqlite3
        from tools import parity_shim
        connect, old = sqlite3.connect, []

        def old_shim_first(*args, **kwargs):
            # As the new shim, having found no database, opens one, an old shim creates it, writes a hash and stays
            # connected: the new shim must go on with that file, never replace it.
            if not old:
                old.append(connect(parity_shim.CACHE / "hashes.sqlite"))
                for statement in OLD_SHIM:
                    old[0].execute(statement)
                old[0].execute("insert into hashes values ('kept', 1, 1, 'x')")
                old[0].commit()
            return connect(*args, **kwargs)

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"):
            parity_shim.CACHE.mkdir()
            try:
                with mock.patch.object(sqlite3, "connect", side_effect=old_shim_first):
                    hashes = parity_shim.Hashes()
                self.assertEqual(hashes.db.execute("select sha from hashes where path = 'kept'").fetchone(), ("x",))
                hashes.db.close()
                # Nothing beside the database but its lock: no temporary copy.
                self.assertEqual([entry.name for entry in parity_shim.CACHE.iterdir()
                                  if not entry.name.startswith("hashes.sqlite")], ["hashes.lock"])
            finally:
                for connection in old:
                    connection.close()

    def test_a_shim_from_before_the_lock_holding_the_database_is_waited_for(self):
        import sqlite3, threading
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"):
            parity_shim.CACHE.mkdir()
            old = sqlite3.connect(parity_shim.CACHE / "hashes.sqlite", check_same_thread=False, isolation_level=None)
            old.execute("begin exclusive")  # mid-way through its own creation: nobody else may read or switch it
            connect, attempts, released = sqlite3.connect, [], threading.Event()

            def release():
                old.execute("commit")
                released.set()

            def counted(*args, **kwargs):
                # The lock is released only after the shim's first attempt has met it.
                attempts.append(time.monotonic())
                if len(attempts) == 1:
                    threading.Timer(0.5, release).start()
                return connect(*args, **kwargs)

            try:
                with mock.patch.object(sqlite3, "connect", side_effect=counted):
                    hashes = parity_shim.Hashes()
                # It waited for the release, through more than one attempt, instead of failing or giving up.
                self.assertTrue(released.is_set())
                self.assertGreater(len(attempts), 1)
                self.assertTrue(parity_shim.in_wal_mode(parity_shim.CACHE / "hashes.sqlite"))
                hashes.db.close()
            finally:
                released.wait(5)
                old.close()

    def test_a_database_locked_throughout_gives_up_soon_after_the_deadline(self):
        import sqlite3
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"), \
                mock.patch.object(parity_shim, "INIT_SECONDS", 0.5):
            parity_shim.CACHE.mkdir()
            old = sqlite3.connect(parity_shim.CACHE / "hashes.sqlite", isolation_level=None)
            old.execute("begin exclusive")
            try:
                started = time.monotonic()
                with self.assertRaises(parity_shim.Uncached):
                    parity_shim.Hashes()
                # Each wait on SQLite is SETUP_BUSY_SECONDS at most, checked against the deadline between them.
                self.assertLess(time.monotonic() - started, 0.5 + 1.0)
            finally:
                old.execute("rollback")
                old.close()

    def test_a_lock_held_past_the_deadline_runs_the_oracle_uncached(self):
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"), \
                mock.patch.object(parity_shim, "INIT_SECONDS", 0.3):
            parity_shim.CACHE.mkdir()
            with open(parity_shim.CACHE / "hashes.lock", "a+b") as holder:
                self.assertTrue(parity_shim.file_lock.lock(holder))
                started = time.monotonic()
                with self.assertRaises(parity_shim.Uncached):
                    parity_shim.Hashes()
                self.assertLess(time.monotonic() - started, 5)
        # main() then runs the real program, as PARITY_JAVA_CACHE=0 does, instead of failing the harness.
        with tempfile.TemporaryDirectory() as temp:
            decisions = Path(temp) / "decisions.jsonl"
            with mock.patch.object(parity_shim, "Hashes", side_effect=parity_shim.Uncached("held")), \
                    mock.patch.object(parity_shim, "real", return_value="/real/java"), \
                    mock.patch.object(parity_shim, "enter"), \
                    mock.patch.object(sys, "argv", ["java", "-cp", "x", "A"]), \
                    mock.patch.dict(os.environ, {"PARITY_JAVA_CACHE": "", "PARITY_SHIM_DECISIONS": str(decisions),
                                                 "PARITY_SHIM_HARNESS": "food"}), \
                    mock.patch.object(parity_shim.os, "execv", side_effect=SystemExit(0)) as execv, \
                    mock.patch("builtins.print") as said, self.assertRaises(SystemExit):
                parity_shim.main()
            self.assertEqual(json.loads(decisions.read_text()),
                             {"harness": "food", "program": "java", "reason": "passthrough-uncached", "seconds": None,
                              "call": None})
        execv.assert_called_once_with("/real/java", ["/real/java", "-cp", "x", "A"])
        self.assertIn("runs without it", said.call_args.args[0])

    def test_a_warm_cache_takes_no_lock(self):
        import hashlib
        from tools import parity_shim
        expected = hashlib.sha256(Path(parity_shim.__file__).read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"):
            parity_shim.Hashes().db.close()
            with mock.patch.object(parity_shim.file_lock, "lock", side_effect=AssertionError("locked")):
                hashes = parity_shim.Hashes()
            self.assertEqual([hashes.file(parity_shim.__file__) for _ in range(2)], [expected, expected])
            hashes.db.close()

    def test_a_wal_database_without_its_table_is_completed(self):
        import sqlite3
        from tools import parity_shim
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity_shim, "CACHE", Path(temp) / "cache"):
            parity_shim.CACHE.mkdir()
            db = sqlite3.connect(parity_shim.CACHE / "hashes.sqlite")
            db.execute(OLD_SHIM[0])  # an older shim stopped here, before its create table
            db.close()
            hashes = parity_shim.Hashes()
            self.assertEqual(len(hashes.file(parity_shim.__file__)), 64)
            hashes.db.close()


class LargeOutputTests(unittest.TestCase):
    def test_a_pass_clears_only_its_own_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            local, state = Path(tmp) / ".local", Path(tmp) / "state"
            own, shared = local / "decorated-region-parity", local / "chunk-generation-parity"
            for directory in (own, shared):
                directory.mkdir(parents=True)
                (directory / "dump.txt").write_bytes(b"x" * 16)
            trace = {"commands": [["java", "-d", str(shared / "classes")], ["probe", str(own / "dump.txt")]]}
            with mock.patch.multiple(parity, LOCAL=local, STATE_DIR=state, LARGE_OUTPUT=8):
                parity.outputs_listing("decorated_region", fresh=True)
                parity.clear_large_outputs("decorated_region", trace)
            self.assertFalse((own / "dump.txt").exists())
            # chunk_generation may still be writing there.
            self.assertTrue((shared / "dump.txt").exists())

    def test_rust_named_directories_and_big_oracle_outputs_go_too(self):
        with tempfile.TemporaryDirectory() as tmp:
            local, state = Path(tmp) / ".local", Path(tmp) / "state"
            own = local / "rust-state-provider-parity"
            own.mkdir(parents=True)
            for name, size in (("rust.txt", 16), ("server.txt", 16), ("client.txt", 64), ("report.json", 4)):
                (own / name).write_bytes(b"x" * size)
            trace = {"commands": [["java", "Oracle", str(own)], ["probe", "fixture", str(own)]]}
            with mock.patch.multiple(parity, LOCAL=local, STATE_DIR=state, LARGE_OUTPUT=8, KEPT_ORACLE_OUTPUT=32):
                listing = parity.outputs_listing("state_provider#2", fresh=True)
                listing.write_text(f"{own / 'server.txt'}\n{own / 'client.txt'}\n")
                parity.clear_large_outputs("state_provider#2", trace)
            self.assertEqual(sorted(path.name for path in own.iterdir()), ["report.json", "server.txt"])


class FakeExecutor:
    """Runs nothing: each start of a harness takes the next status from its script, a list."""
    name, dead, max_peak = "fake", False, None

    def __init__(self):
        self.started, self.finished = [], []

    def free(self):
        return 1

    def accepts(self, name):
        return True

    def fits(self, expected, only_here=True):
        return True

    def start(self, label, script, reuse, expected, shard=None):
        self.started.append((label, reuse))
        self.finished.append((label, {"status": script.pop(0), "seconds": 1.0, "dependencies": []}))
        return True

    def poll(self):
        finished, self.finished = self.finished, []
        return finished


class KnownFlakyTests(unittest.TestCase):
    def run_harnesses(self, scripts, flaky, rounds=None):
        executor = FakeExecutor()
        state = {"harnesses": {name: {"status": "pass", "oracle_reuse": True} for name in scripts}}
        with tempfile.TemporaryDirectory() as tmp, mock.patch.multiple(parity, STATE_DIR=Path(tmp), SHARED=Path(tmp)), \
                mock.patch.object(parity, "save_state"), mock.patch.object(parity, "time") as clock:
            clock.monotonic.return_value = 0.0
            results, _ = parity.run(scripts, state, None, "env", [executor], True, rounds, flaky)
        return results, executor.started

    def test_a_known_flaky_failure_runs_once_more_from_scratch_and_says_so(self):
        results, started = self.run_harnesses({"dedicated_server": ["fail", "pass"]}, {"dedicated_server": "timer"})
        self.assertEqual(results["dedicated_server"]["status"], "pass")
        self.assertEqual(results["dedicated_server"]["retried"], "known flaky")
        # The first run reused oracle results; a known flaky harness doesn't lose that by flaking.
        self.assertTrue(results["dedicated_server"]["oracle_reuse"])
        self.assertEqual(started, [("dedicated_server", True), ("dedicated_server", False)])

    def test_only_once(self):
        results, started = self.run_harnesses({"dedicated_server": ["fail", "fail"]}, {"dedicated_server": "timer"})
        self.assertEqual(results["dedicated_server"]["status"], "fail")
        self.assertEqual(len(started), 2)

    def test_other_harnesses_keep_only_the_reused_oracle_retry(self):
        results, started = self.run_harnesses({"food": ["fail", "pass"]}, {"dedicated_server": "timer"})
        self.assertEqual((results["food"]["status"], results["food"]["retried"]), ("pass", "reused oracle results"))
        self.assertEqual(started, [("food", True), ("food", False)])
        self.assertFalse(results["food"]["oracle_reuse"])

    def test_only_hosted_runs_keep_the_first_log_of_an_ordinary_retry(self):
        for keep, logs in (("1", ["food.first-run.log"]), ("", ["food.log"])):
            with self.subTest(keep=keep), tempfile.TemporaryDirectory() as tmp, \
                    mock.patch.dict(os.environ, {"PARITY_KEEP_FIRST_LOGS": keep}), \
                    mock.patch.multiple(parity, STATE_DIR=Path(tmp), SHARED=Path(tmp)), \
                    mock.patch.object(parity, "save_state"), mock.patch.object(parity, "time") as clock, \
                    mock.patch("builtins.print"):
                clock.monotonic.return_value = 0.0
                (Path(tmp) / "logs").mkdir()
                (Path(tmp) / "logs/food.log").write_text("the first run")
                state = {"harnesses": {"food": {"status": "pass", "oracle_reuse": True}}}
                results, _ = parity.run({"food": ["fail", "pass"]}, state, None, "env", [FakeExecutor()], True)
                self.assertEqual(results["food"]["retried"], "reused oracle results")
                self.assertEqual(sorted(path.name for path in (Path(tmp) / "logs").iterdir()), logs)

    def test_repeated_runs_get_no_retry(self):
        results, started = self.run_harnesses({"dedicated_server": ["pass", "fail"]}, {"dedicated_server": "timer"},
                                              rounds={"dedicated_server": 2})
        self.assertEqual((results["dedicated_server"]["status"], len(started)), ("fail", 2))

    def test_retries_reach_the_job_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            summary = Path(tmp) / "summary.md"
            with mock.patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": str(summary)}), \
                    mock.patch("builtins.print"):
                parity.report_retries({"dedicated_server": {"status": "pass", "retried": "known flaky"},
                                       "food": {"status": "pass"}}, {"dedicated_server": "timer"})
            text = summary.read_text()
        self.assertIn("RETRIED dedicated_server: failed, then passed on its one retry (known flaky: timer", text)
        self.assertNotIn("food", text)

    def test_the_known_flaky_list_has_a_reason_per_harness(self):
        flaky = parity.known_flaky(parity.host_config())
        self.assertTrue(all(isinstance(reason, str) and len(reason) > 20 for reason in flaky.values()))
        self.assertTrue(set(flaky) <= {parity.base_name(name) for name in parity.harnesses()})



class SoloTests(unittest.TestCase):
    """Harnesses driving game windows run alone on a small machine (playhorizon's 4-CPU runner)."""

    def executor(self, running):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity, "STATE_DIR", Path(tmp)):
            local = parity.LocalExecutor(4, 1 << 40, 60)
        local.solo = {"remote_fireworks"}
        local.running = {label: {} for label in running}
        return local

    def test_a_solo_harness_waits_for_an_empty_machine_and_then_holds_it(self):
        busy = self.executor(["food"])
        self.assertFalse(busy.accepts("remote_fireworks"))
        self.assertTrue(busy.accepts("light"))
        self.assertEqual(busy.free(), 3)
        self.assertTrue(self.executor([]).accepts("remote_fireworks"))
        alone = self.executor(["remote_fireworks~2"])
        self.assertEqual(alone.free(), 0)

    def test_only_small_machines_and_listed_harnesses(self):
        config = {"solo": {"remote_fireworks": "two game windows"}}
        with mock.patch.dict(os.environ, {"PARITY_SOLO": "1"}):
            self.assertEqual(parity.solo_harnesses(config), {"remote_fireworks"})
        with mock.patch.dict(os.environ, {"PARITY_SOLO": "0"}):
            self.assertEqual(parity.solo_harnesses(config), set())
        with mock.patch.dict(os.environ, {"PARITY_SOLO": ""}), mock.patch.object(parity, "SOLO_CPUS", 10 ** 6):
            self.assertEqual(parity.solo_harnesses(config), {"remote_fireworks"})
        solo = parity.host_config().get("solo", {})
        self.assertTrue(solo and set(solo) <= {parity.base_name(name) for name in parity.harnesses()})


class ClashTests(unittest.TestCase):
    """On Windows, harnesses that build one program with different selections don't run at once."""
    PACKAGES = {("bin", "horizon-client"): "horizon-client", ("bin", "horizon-ctl"): "horizon-client-lib",
                ("example", "client_probe"): "horizon-client-lib"}
    SERVER = ["bin", "horizon-client", "fast", ["-p", "horizon-client"], False]
    BOTH = ["bins", "", "fast", ["-p", "horizon-client", "-p", "horizon-client-lib"], False]
    PROBE = ["example", "client_probe", "dev", ["-p", "horizon-client-lib"], True]

    def test_a_run_naming_every_bin_builds_those_of_its_packages(self):
        self.assertEqual(parity.executables([self.BOTH], self.PACKAGES), {
            ("bin", "horizon-client", "fast"): (("-p", "horizon-client", "-p", "horizon-client-lib"), False),
            ("bin", "horizon-ctl", "fast"): (("-p", "horizon-client", "-p", "horizon-client-lib"), False)})

    def test_another_selection_of_a_running_harness_program_waits(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity, "STATE_DIR", Path(tmp)):
            local = parity.LocalExecutor(4, 1 << 40, 60)
        local.builds = {name: parity.executables(built, self.PACKAGES) for name, built in {
            "server": [self.SERVER], "same": [self.SERVER, self.PROBE], "both": [self.BOTH], "probe": [self.PROBE],
            "python_only": []}.items()}
        local.running = {"server": {}}
        self.assertFalse(local.accepts("both"))  # it would relink horizon-client.exe while "server" runs it
        self.assertTrue(local.accepts("same"))  # Cargo finds it fresh
        self.assertTrue(local.accepts("probe"))
        self.assertTrue(local.accepts("python_only"))
        self.assertTrue(local.accepts("never_ran"))
        local.running = {"both~2": {}}
        self.assertFalse(local.accepts("server"))
        local.running = {}
        self.assertTrue(local.accepts("both"))


if __name__ == "__main__":
    unittest.main()


class WorkspaceManifestTests(unittest.TestCase):
    """A crate split's Cargo.toml and Cargo.lock changes don't select every harness; a dependency change still does."""
    CLIENT = '[package]\nname = "horizon-client"\n\n[dependencies]\nserde = "1"\nwgpu = { version = "27", features = ["x"] }\n'
    LOCK = ('version = 4\n\n[[package]]\nname = "horizon-client"\nversion = "0.1.0"\ndependencies = [\n "serde",\n]\n\n'
            '[[package]]\nname = "serde"\nversion = "1.0.1"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n')

    def verdict(self, base, head):
        """base, head: {path: text or None}; the manifests workspace_only_manifests leaves out."""
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "git_show", lambda _base, path: base.get(path)):
            for path, text in head.items():
                if text is not None:
                    Path(temp, path).parent.mkdir(parents=True, exist_ok=True)
                    Path(temp, path).write_text(text)
            return parity.workspace_only_manifests(set(head) | set(base), "base")

    def test_a_crate_split_is_left_out(self):
        command = '[package]\nname = "horizon-command"\n\n[dependencies]\nserde = "1"\nhorizon-core = { path = "../horizon-core" }\n'
        client = self.CLIENT + 'horizon-command = { path = "../horizon-command" }\n'
        lock = self.LOCK.replace('[[package]]\nname = "serde"', '[[package]]\nname = "horizon-command"\nversion = "0.1.0"\n\n'
                                 '[[package]]\nname = "serde"')
        base = {"Cargo.toml": '[workspace]\nmembers = ["crates/horizon-client"]\nresolver = "2"\n',
                "crates/horizon-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK}
        head = {"Cargo.toml": '[workspace]\nmembers = ["crates/horizon-client", "crates/horizon-command"]\nresolver = "2"\n',
                "crates/horizon-client/Cargo.toml": client, "crates/horizon-command/Cargo.toml": command, "Cargo.lock": lock}
        self.assertEqual(self.verdict(base, head), set(head))

    def test_a_dependency_change_still_counts(self):
        base = {"crates/horizon-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK}
        bumped = {"crates/horizon-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK.replace("1.0.1", "1.0.2")}
        self.assertEqual(self.verdict(base, bumped), set())
        feature = {"crates/horizon-client/Cargo.toml": self.CLIENT.replace('["x"]', '["x", "y"]')}
        self.assertEqual(self.verdict({"crates/horizon-client/Cargo.toml": self.CLIENT}, feature), set())
        resolver = {"Cargo.toml": '[workspace]\nmembers = ["a"]\nresolver = "3"\n'}
        self.assertEqual(self.verdict({"Cargo.toml": '[workspace]\nmembers = ["a"]\nresolver = "2"\n'}, resolver), set())

    def test_a_hash_inside_a_quoted_value_is_not_a_comment(self):
        git = '[dependencies]\nserde_json = {{ git = "https://example.invalid/repo.git#{}" }}\n'
        self.assertNotEqual(parity.manifest_lines(git.format("rev1")), parity.manifest_lines(git.format("rev2")))
        self.assertEqual(parity.manifest_lines("[dependencies]\nserde_json = \"1.0\" # note\nx = '#a' # b\n"),
                         {("[dependencies]", 'serde_json = "1.0"'), ("[dependencies]", "x = '#a'")})
        path = "crates/foo/Cargo.toml"
        self.assertEqual(self.verdict({path: git.format("rev1")}, {path: git.format("rev2")}), set())

    def test_a_feature_of_a_workspace_crate_counts_through_its_sources(self):
        items = '[package]\nname = "horizon-items"\n\n[features]\ntest-support = []\n'
        client = self.CLIENT + '\n[dev-dependencies]\nhorizon-items = { path = "../horizon-items", features = ["test-support"] }\n'
        head = {"crates/horizon-items/Cargo.toml": items, "crates/horizon-client/Cargo.toml": client,
                "crates/horizon-items/src/lib.rs": '#[cfg(feature = "test-support")]\npub fn for_tests() {}\n'}
        base = {"crates/horizon-client/Cargo.toml": self.CLIENT}
        # The new crate's files are all changed, the code the feature gates with them.
        self.assertEqual(self.verdict(base, head), {"crates/horizon-items/Cargo.toml", "crates/horizon-client/Cargo.toml"})
        # Turning on a feature of a crate whose gated code didn't change selects as before.
        world = '[package]\nname = "horizon-world"\n\n[features]\nextra = ["more"]\nmore = []\n'
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            Path(temp, "crates/horizon-world/src").mkdir(parents=True)
            Path(temp, "crates/horizon-world/Cargo.toml").write_text(world)
            Path(temp, "crates/horizon-world/src/lib.rs").write_text('#[cfg(feature = "more")]\nfn x() {}\n')
            self.assertFalse(parity.feature_sources_changed("horizon-world", "extra", {"crates/horizon-client/Cargo.toml"}))
            self.assertTrue(parity.feature_sources_changed("horizon-world", "extra", {"crates/horizon-world/src/lib.rs"}))
            self.assertFalse(parity.feature_sources_changed("horizon-nothing", "extra", set()))
        # A feature that reaches an external package counts.
        optional = {"crates/horizon-client/Cargo.toml": self.CLIENT + '\n[features]\nfast = ["wgpu/x"]\n'}
        self.assertEqual(self.verdict({"crates/horizon-client/Cargo.toml": self.CLIENT}, optional), set())

    def test_external_feature_sources_are_not_discarded(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            root = Path(temp)
            (root / "crates/core").mkdir(parents=True)
            (root / "shared").mkdir()
            (root / "crates/core/Cargo.toml").write_text(
                '[package]\nname = "core"\n[lib]\npath = "../../shared/lib.rs"\n'
                '[features]\nextra = []\n')
            (root / "shared/lib.rs").write_text('#[cfg(feature = "extra")]\nfn extra() {}\n')
            self.assertFalse(parity.feature_sources_changed("core", "extra", {"crates/core/Cargo.toml"}))

    def test_without_a_base_everything_counts(self):
        self.assertEqual(parity.workspace_only_manifests({"Cargo.lock"}, None), set())


# A harness that leaves a child behind: the child locks a file, says so, and sleeps; the harness then exits or hangs.
# The lock frees once the child is dead, on every platform (flock, or msvcrt's byte lock).
LEAVING_HARNESS = """
import subprocess, sys, time
from pathlib import Path
held = Path(sys.argv[0]).with_name("held")
child = subprocess.Popen([sys.executable, "-c", '''
import sys, time
sys.path.insert(0, sys.argv[1])
from tools import file_lock
handle = open(sys.argv[2], "w")
file_lock.lock(handle)
open(sys.argv[2] + ".ready", "w").close()
time.sleep(120)
''', str(Path({root!r})), str(held)], stdout=subprocess.DEVNULL)
while not Path(str(held) + ".ready").exists():
    time.sleep(0.05)
if {hang!r}:
    time.sleep(120)
"""


def child_released(held, seconds=30):
    """Whether the left child's lock on held frees within seconds (it is dead)."""
    from tools import file_lock
    deadline = time.monotonic() + seconds
    with open(held, "a") as handle:
        while time.monotonic() < deadline:
            if file_lock.lock(handle, wait=False):
                return True
            time.sleep(0.1)
    return False


class LeftProcessTests(unittest.TestCase):
    """What a harness starts ends with it, whether it exits, crashes or times out (tools/process_tree.py)."""

    def run_harness(self, hang, timeout):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            script = tmp / "test_leaving_parity.py"
            script.write_text(LEAVING_HARNESS.format(root=str(parity.ROOT), hang=hang), encoding="utf-8")
            with mock.patch.object(parity, "STATE_DIR", tmp / "state"), mock.patch.object(parity, "SHARED", tmp / "shared"):
                local = parity.LocalExecutor(1, 1 << 40, timeout)
                self.assertTrue(local.start("leaving", script, False, 0))
                finished, deadline = [], time.monotonic() + 120
                while not finished and time.monotonic() < deadline:
                    finished = local.poll()
                    time.sleep(0.1)
                local.stop()
            self.assertEqual([name for name, *_ in finished], ["leaving"])
            self.assertTrue((tmp / "held.ready").exists())
            self.assertTrue(child_released(tmp / "held"), "the harness's child outlived it")
            return finished[0]

    @unittest.skipUnless(os.name == "nt" or hasattr(os, "waitid"), "macOS before Python 3.13 has no os.waitid")
    def test_a_finished_harness_takes_what_it_left_running(self):
        _, _, passed, *_ = self.run_harness(hang=False, timeout=120)
        self.assertTrue(passed)

    def test_a_timed_out_harness_takes_what_it_started(self):
        _, _, passed, *_ = self.run_harness(hang=True, timeout=5)
        self.assertFalse(passed)
