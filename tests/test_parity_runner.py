from pathlib import Path
import os
import tempfile
import unittest
from unittest import mock

from tools import parity


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
    def test_a_sharded_harness_runs_as_numbered_entries(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            (Path(temp) / "tools").mkdir()
            (Path(temp) / "tools/test_x_parity.py").write_text("PARITY_SHARDS = 3\n")
            (Path(temp) / "tools/test_y_parity.py").write_text("")
            self.assertEqual(sorted(parity.harnesses()), ["x#1", "x#2", "x#3", "y"])
            self.assertEqual(parity.SHARDS["x#2"], "2/3")
            self.assertEqual(parity.base_name("x#2"), "x")


@unittest.skipIf(os.name == "nt", "oracle PATH shimming uses Unix executable symlink semantics")
class OracleReuseTests(unittest.TestCase):
    """tools/parity_shim.py replays a run only when its inputs and outputs are as recorded."""

    def setUp(self):
        import os, subprocess, sys
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.bin, self.shim = root / "bin", root / "shim"
        self.bin.mkdir(); self.shim.mkdir()
        # A fake oracle: copies its input to its output and counts its runs.
        fake = self.bin / "java"
        fake.write_text(f"#!{sys.executable}\nimport sys, pathlib\n"
                        "runs = pathlib.Path(sys.argv[1]).parent / 'runs'\n"
                        "runs.write_text(str(int(runs.read_text() or 0) + 1) if runs.exists() else '1')\n"
                        "pathlib.Path(sys.argv[2]).write_text(pathlib.Path(sys.argv[1]).read_text())\nprint('done')\n")
        fake.chmod(0o755)
        (self.shim / "java").symlink_to(Path(parity.__file__).with_name("parity_shim.py"))
        self.input, self.output = root / "work" / "in.txt", root / "work" / "out.txt"
        self.input.parent.mkdir()
        self.input.write_text("a")
        python = Path(sys.executable).parent
        self.env = dict(os.environ, PATH=f"{self.shim}:{self.bin}:{python}", PARITY_SHIM_REAL_PATH=str(self.bin),
                        PARITY_SHIM_CACHE=str(root / "cache"))
        self.subprocess = subprocess

    def tearDown(self):
        self.temp.cleanup()

    def oracle(self):
        result = self.subprocess.run(["java", str(self.input), str(self.output)], env=self.env, capture_output=True, text=True,
                                     stdin=self.subprocess.DEVNULL, check=True)
        self.assertEqual(result.stdout, "done\n")
        return int((self.input.parent / "runs").read_text())

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


if __name__ == "__main__":
    unittest.main()
