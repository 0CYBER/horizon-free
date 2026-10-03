import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import zipfile

from tools import build_pack, compare_benchmarks, runtime


class PackTests(unittest.TestCase):
    def test_original_mod_upgrade_preserves_backup_and_other_mods(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            mods = root / "instance/mods"
            mods.mkdir(parents=True)
            old = mods / "horizon-lab-0.2.0.jar"
            with zipfile.ZipFile(old, "w") as archive:
                archive.writestr("fabric.mod.json", json.dumps({"id": "horizonlab"}))
            old_bytes = old.read_bytes()
            other = mods / "other.jar"
            other.write_bytes(b"another mod")
            new = root / "horizon-lab-0.3.0.jar"
            new.write_bytes(b"new original fixture")
            runtime.install_original_mod(new, mods)
            self.assertFalse(old.exists())
            self.assertEqual((mods / new.name).read_bytes(), new.read_bytes())
            self.assertEqual(other.read_bytes(), b"another mod")
            backups = list((mods.parent / ".horizon-mod-backups").glob("*.jar"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_bytes(), old_bytes)

    def test_original_mod_upgrade_refuses_unrelated_named_jar(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with zipfile.ZipFile(root / "horizon-lab-0.1.0.jar", "w") as archive:
                archive.writestr("fabric.mod.json", json.dumps({"id": "someoneelse"}))
            new = root / "horizon-lab-0.3.0.jar"
            new.write_bytes(b"new original fixture")
            with self.assertRaisesRegex(ValueError, "another mod"):
                runtime.install_original_mod(new, root)

    def test_every_profile_has_correct_game_and_exact_dependency_closure(self):
        lock = build_pack.read_lock()
        for profile in build_pack.PROFILES.values():
            build_pack.validate_selection(lock, profile["mods"])

    def test_missing_exact_iris_sodium_dependency_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Missing pinned dependency"):
            build_pack.validate_selection(build_pack.read_lock(), ["iris"])

    def test_wrong_game_or_download_path_is_rejected(self):
        for field, value in [("game_versions", ["1.21"]), ("filename", "../bad.jar"), ("url", "http://unsafe.invalid")]:
            lock = copy.deepcopy(build_pack.read_lock())
            lock["projects"]["sodium"][field] = value
            with self.assertRaises(ValueError):
                build_pack.validate_selection(lock, ["sodium"])

    def test_portable_pack_contains_only_original_embedded_mod_and_download_references(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "packs").mkdir()
            (root / "packs/mods.lock.json").write_text(json.dumps(build_pack.read_lock()))
            (root / "packs/config").mkdir()
            (root / "packs/config/m2-balanced.toml").write_text("[client.advanced.graphics.quality]\nlodChunkRenderDistanceRadius = 128\n")
            original = root / "original.jar"
            original.write_bytes(b"original fixture")
            with mock.patch.object(build_pack, "ROOT", root):
                pack = build_pack.build("m2-balanced", original)
            with zipfile.ZipFile(pack) as archive:
                names = archive.namelist()
                self.assertEqual([n for n in names if n.endswith(".jar")],
                                 [f"overrides/mods/horizon-lab-{build_pack.read_lock()['pack_version']}.jar"])
                index = json.loads(archive.read("modrinth.index.json"))
                self.assertEqual(index["dependencies"]["minecraft"], "26.3")
                options = dict(line.split(":", 1) for line in archive.read("overrides/options.txt").decode().splitlines())
                # A missing/currently wrong version invokes the game's graphics API reset migration.
                self.assertEqual(options["version"], "5023")
                self.assertEqual(json.loads(options["preferredGraphicsBackend"]), "opengl")
                for entry in index["files"]:
                    self.assertEqual(set(entry["hashes"]), {"sha1", "sha512"})
                    self.assertTrue(entry["downloads"][0].startswith("https://cdn.modrinth.com/"))

    def test_native_library_rules_choose_only_matching_operating_system(self):
        library = {"rules": [{"action": "allow", "os": {"name": "osx", "arch": "aarch64|arm64"}}]}
        self.assertTrue(runtime.library_allowed(library, "osx", "aarch64"))
        self.assertFalse(runtime.library_allowed(library, "linux", "aarch64"))
        self.assertFalse(runtime.library_allowed(library, "osx", "x86_64"))

    def test_native_architecture_is_selected_even_with_os_only_rules(self):
        intel = {"name": "org.lwjgl:lwjgl:3.4.3:natives-macos"}
        arm = {"name": "org.lwjgl:lwjgl:3.4.3:natives-macos-arm64"}
        self.assertTrue(runtime.native_library_matches(arm, "aarch64"))
        self.assertFalse(runtime.native_library_matches(intel, "aarch64"))
        self.assertTrue(runtime.native_library_matches(intel, "x86_64"))
        self.assertFalse(runtime.native_library_matches(arm, "AMD64"))


class ComparisonTests(unittest.TestCase):
    def report(self, kind="frames"):
        return {"schema": 1, "kind": kind, "elapsed_seconds": 120, "framebuffer_width": 1920,
                "stats": {"count": 1000, "meanRate": 100, "lowOnePercentRate": 50,
                          "meanMs": 10, "p95Ms": 15, "p99Ms": 20, "maxMs": 30}}

    def test_rate_improvement_and_setting_mismatch(self):
        baseline = self.report()
        candidate = copy.deepcopy(baseline)
        candidate["stats"]["meanRate"] = 150
        candidate["framebuffer_width"] = 1280
        result = compare_benchmarks.compare(baseline, candidate)
        self.assertEqual(result["metrics"][0]["change_percent"], 50)
        self.assertIn("framebuffer_width", result["mismatches"][0])

    def test_no_samples_and_mixed_kinds_are_rejected(self):
        empty = self.report()
        empty["stats"]["count"] = 0
        for other in [empty, self.report("ticks")]:
            with self.assertRaises(ValueError):
                compare_benchmarks.compare(self.report(), other)


if __name__ == "__main__":
    unittest.main()
