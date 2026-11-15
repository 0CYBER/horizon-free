"""Crate renames must not rename player data paths or the separate registry repo."""
from contextlib import redirect_stdout
import importlib.util
import io
from pathlib import Path
import shutil
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("rename_crate", Path(__file__).resolve().parents[1] / "tools/rename_crate.py")
rename = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rename)


class RenameCrateTests(unittest.TestCase):
    def test_manifest_and_crate_paths_change_without_touching_player_paths(self):
        source = '''[package]
name = "horizon-mods"
[dependencies]
horizon-mods = { path = "../horizon-mods" }
[dev-dependencies.horizon-mods]
path = "../crates/horizon-mods"
# Put files in horizon-mods/ and enable them in horizon-mods.txt.
'''
        expected = '''[package]
name = "horizon-mod-package"
[dependencies]
horizon-mod-package = { path = "../horizon-mod-package" }
[dev-dependencies.horizon-mod-package]
path = "../crates/horizon-mod-package"
# Put files in horizon-mods/ and enable them in horizon-mods.txt.
'''
        self.assertEqual(rename.rewrite("crates/horizon-mods/Cargo.toml", source), expected)
        self.assertEqual(rename.destination("crates/horizon-mods/tests/file.hmod"),
                         "crates/horizon-mod-package/tests/file.hmod")
        self.assertEqual(rename.destination("crates/horizon-mods-other/src/lib.rs"),
                         "crates/horizon-mods-other/src/lib.rs")
        self.assertEqual(rename.rewrite("crates/horizon-mod-registry/tests/submit.rs", '../horizon-mods/tests/index'),
                         '../horizon-mod-package/tests/index')

    def test_rust_library_identifiers_and_link_arguments_change(self):
        source = '''use horizon_mods::{Content, Side};
mod horizon_mods { fn fixture() {} }
let link = "horizon_mods={}";
let module = horizon_mods::Package;
fn horizon_mods_test_name() {}
let folder = "horizon-mods";
let registry = "https://github.com/horizon-rust/horizon-mods";
'''
        expected = '''use horizon_mod_package::{Content, Side};
mod horizon_mod_package { fn fixture() {} }
let link = "horizon_mod_package={}";
let module = horizon_mod_package::Package;
fn horizon_mods_test_name() {}
let folder = "horizon-mods";
let registry = "https://github.com/horizon-rust/horizon-mods";
'''
        self.assertEqual(rename.rewrite("launcher/src/native_mods/prebuilt.rs", source), expected)

    def test_generated_manifest_packages_do_not_rename_player_or_negative_strings(self):
        source = '''for name in [
    "horizon-mods",
] { let dependency = path_dep(&format!("../{name}")); }
let folder = "horizon-mods";
assert!(!dependencies.contains_key("horizon-mods"));
'''
        expected = '''for name in [
    "horizon-mod-package",
] { let dependency = path_dep(&format!("../{name}")); }
let folder = "horizon-mods";
assert!(!dependencies.contains_key("horizon-mods"));
'''
        self.assertEqual(rename.rewrite("launcher/src/native_mods/api.rs", source), expected)
        self.assertEqual(rename.rewrite("launcher/src/export.rs", source), source)

    @unittest.skipUnless(shutil.which("rustfmt"), "requires rustfmt")
    def test_verify_rejects_an_unchanged_file_that_still_needs_the_mapping(self):
        with TemporaryDirectory() as directory, patch.object(rename, "ROOT", Path(directory)):
            crate = rename.ROOT / rename.OLD_PATH
            crate.mkdir(parents=True)
            (crate / "Cargo.toml").write_text('[package]\nname = "horizon-mods"\n')
            consumer = rename.ROOT / "consumer.rs"
            consumer.write_text("use horizon_mods::Manifest;\n")
            rename.run("git", "init", "-q")
            rename.run("git", "add", "--", rename.OLD_PATH, "consumer.rs")
            rename.run("git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                       "commit", "-qm", "Baseline")
            rename.run("git", "mv", rename.OLD_PATH, rename.NEW_PATH)
            (rename.ROOT / rename.NEW_PATH / "Cargo.toml").write_text(
                '[package]\nname = "horizon-mod-package"\n', newline="\n")
            with self.assertRaisesRegex(AssertionError, "unexpected content change: consumer.rs"):
                rename.verify("HEAD")
            consumer.write_text("use horizon_mod_package::Manifest;\n", newline="\n")
            with redirect_stdout(io.StringIO()):
                rename.verify("HEAD")

    def test_book_filters_and_translations_obey_the_boundary(self):
        source = '{{#include ../../crates/horizon-mods/src/lib.rs}}\n(crates/horizon-mods)\ncrates/horizon-mods/**\n'
        expected = '{{#include ../../crates/horizon-mod-package/src/lib.rs}}\n(crates/horizon-mod-package)\ncrates/horizon-mod-package/**\n'
        self.assertEqual(rename.rewrite("docs/modding/src/reference.md", source), expected)
        translation = '{"native.mods": "In horizon-mods; internal type horizon_mods::Package"}'
        self.assertEqual(rename.rewrite("crates/horizon-client/assets/horizon/lang/en_us.json", translation), translation)
        self.assertEqual(rename.rewrite("roadmap/roadmap.toml", 'dev = "horizon-mods/native/"'),
                         'dev = "horizon-mods/native/"')


if __name__ == "__main__":
    unittest.main()
