import shutil
import subprocess
import unittest
from unittest import mock

from tools.ci.test_scope import NOT_READS, READERS, SCANNED, Workspace, cargo_metadata, flags, reads, under


def package(name, folder, *dependencies, source=None):
    return {
        "name": name,
        "manifest_path": f"/repo/{folder}/Cargo.toml",
        "targets": [{"src_path": f"/repo/{source or folder + '/src'}/lib.rs"}],
        "dependencies": [{"name": dependency} for dependency in dependencies],
    }


def workspace(*packages):
    return Workspace({"workspace_root": "/repo", "packages": list(packages)})


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.graph = workspace(
            package("base", "crates/base"),
            package("mid", "crates/mid", "base"),
            package("top", "crates/top", "mid"),
            package("other", "crates/other"),
            package("twin", "crates/twin", source="crates/base/src"),
        )

    def test_a_change_selects_its_package_and_whatever_depends_on_it(self):
        self.assertEqual(self.graph.scope(["crates/mid/src/a.rs"]), {"mid", "top"})
        self.assertEqual(self.graph.scope(["crates/other/tests/t.rs"]), {"other"})
        self.assertEqual(self.graph.scope(["crates/mid/Cargo.toml", "crates/other/build.rs"]), {"mid", "top", "other"})

    def test_a_package_built_from_another_ones_folder_follows_it(self):
        self.assertEqual(self.graph.scope(["crates/base/assets/a.json"]), {"base", "twin", "mid", "top"})

    def test_everything_selected_is_the_whole_workspace(self):
        self.assertIsNone(self.graph.scope(["crates/base/src/a.rs", "crates/other/src/a.rs"]))

    def test_a_change_outside_every_package_runs_everything(self):
        for path in ["Cargo.lock", "Cargo.toml", ".cargo/config.toml", "clippy.toml", ".github/workflows/ci.yml",
                     ".github/PULL_REQUEST_TEMPLATE.md", "tools/ci/test_scope.py", "tools/pr_ready.py",
                     "native-hooks.json", "third_party/naga/src/lib.rs", "crates/not-a-member/src/lib.rs"]:
            with self.subTest(path=path):
                self.assertIsNone(self.graph.scope(["crates/other/src/a.rs", path]))

    def test_documentation_selects_nothing_unless_a_test_reads_it(self):
        self.assertEqual(self.graph.scope(["README.md", "docs/setup.md", "docs/media/banner.webp"]), set())
        with mock.patch.dict(READERS, {"other": ["docs/api/reference.md"]}):
            self.assertEqual(self.graph.scope(["docs/api/reference.md", "docs/setup.md"]), {"other"})
        # A markdown file in a package's folder may be included by it.
        self.assertEqual(self.graph.scope(["crates/other/README.md"]), {"other"})

    def test_a_reader_is_selected_with_its_dependents(self):
        with mock.patch.dict(READERS, {"mid": ["crates/other/assets"]}):
            self.assertEqual(self.graph.scope(["crates/other/assets/a.json"]), {"other", "mid", "top"})
            self.assertEqual(self.graph.scope(["crates/other/src/a.rs"]), {"other"})

    def test_a_package_that_only_reads_at_run_time_is_selected_alone(self):
        with mock.patch.dict(SCANNED, {"top": ["crates/other"]}):
            self.assertEqual(self.graph.scope(["crates/other/src/a.rs"]), {"other", "top"})
            self.assertEqual(self.graph.scope(["crates/base/src/a.rs"]), {"base", "twin", "mid", "top"})
        with mock.patch.dict(SCANNED, {"mid": ["crates/other"]}):
            self.assertEqual(self.graph.scope(["crates/other/src/a.rs"]), {"other", "mid"})

    def test_the_output_is_the_flags_cargo_takes_or_empty_for_the_workspace(self):
        self.assertEqual(flags({"b", "a"}), "packages=-p a -p b")
        self.assertEqual(flags(None), "packages=")
        self.assertEqual(flags(set()), "packages=")


@unittest.skipUnless(shutil.which("cargo"), "needs cargo metadata")
class RealWorkspaceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = Workspace(cargo_metadata())

    def test_horizon_client_lib_is_built_from_horizon_clients_files(self):
        chosen = self.workspace.scope(["crates/horizon-client/src/game/x.rs"])
        self.assertLessEqual({"horizon-client", "horizon-client-lib", "horizon-client-core"}, chosen)
        self.assertNotIn("horizon-server", chosen)
        self.assertIn("horizon-client-lib", self.workspace.scope(["crates/horizon-client/tests/lan_bot.rs"]))

    def test_every_reader_names_a_package_and_a_path_in_the_tree(self):
        tracked = subprocess.check_output(["git", "ls-files"]).decode().split("\n")
        for name, paths in [*READERS.items(), *SCANNED.items()]:
            self.assertIn(name, self.workspace.dirs)
            for path in paths:
                with self.subTest(reader=name, path=path):
                    self.assertTrue(any(under(file, path) for file in tracked), f"{path} is not in the tree")

    def test_every_cross_crate_read_is_in_the_readers_table(self):
        missing = []
        for name, found in reads(self.workspace).items():
            for path, where in sorted(found.items()):
                listed = READERS.get(name, []) + SCANNED.get(name, [])
                if path in NOT_READS.get(name, ()):
                    continue
                if not (listed if path == "." else any(under(path, entry) for entry in listed)):
                    missing.append(f"{name} reads {path} ({where})")
        self.assertEqual(missing, [], "add each to READERS in tools/ci/test_scope.py, or to NOT_READS if it is no read")

    def test_the_scan_finds_the_reads_the_table_was_built_from(self):
        found = reads(self.workspace)
        self.assertIn("crates/horizon-client/assets/horizon/lang", found["horizon-client-core"])
        self.assertIn("docs/api/server-reference.md", found["horizon-server"])
        self.assertIn("crates/horizon-core", found["horizon-world"])
        self.assertIn("crates", found["horizon-client-core"])


if __name__ == "__main__":
    unittest.main()
