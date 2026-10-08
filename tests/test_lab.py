import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest import mock
import zipfile

from tools import check_public_tree, lab


def minimal_class(source="Example.java"):
    # A valid class-file structure with no fields/methods and one SourceFile attribute.
    def utf8(value):
        data = value.encode()
        return b"\x01" + struct.pack(">H", len(data)) + data
    return (b"\xca\xfe\xba\xbe" + struct.pack(">HHH", 0, 69, 5)
            + utf8("SourceFile") + utf8(source) + utf8("Example") + b"\x07\x00\x03"
            + struct.pack(">HHHHHHHHIH", 1, 4, 0, 0, 0, 0, 1, 1, 2, 2))


class ArtifactTests(unittest.TestCase):
    def test_download_rejects_corruption_without_overwriting_existing_file(self):
        with tempfile.TemporaryDirectory() as temp:
            destination = Path(temp) / "game.jar"
            destination.write_bytes(b"existing")
            with mock.patch("urllib.request.urlopen", return_value=io.BytesIO(b"corrupt")):
                with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
                    lab.download("https://example.invalid/game.jar", destination, "0" * 40)
            self.assertEqual(destination.read_bytes(), b"existing")
            self.assertEqual([p.name for p in Path(temp).iterdir()], ["game.jar"])

    def test_cached_download_is_verified(self):
        with tempfile.TemporaryDirectory() as temp:
            destination = Path(temp) / "game.jar"
            destination.write_bytes(b"verified")
            with mock.patch("urllib.request.urlopen") as request:
                lab.download("https://example.invalid", destination, hashlib.sha1(b"verified").hexdigest())
                request.assert_not_called()

    def test_sourcefile_metadata_covers_inner_classes(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "game.jar"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("demo/Example.class", minimal_class())
                archive.writestr("demo/Example$Inner.class", minimal_class())
            result = lab.inventory(path)
            self.assertEqual(result["class_count"], 2)
            self.assertEqual(result["expected_source_files"], ["demo/Example.java"])

    def test_invalid_class_is_rejected(self):
        for data in [b"oops", b"\xca\xfe\xba\xbe"]:
            with self.assertRaises(ValueError):
                lab.ClassReader(data).metadata()

    def test_server_bundler_extracts_actual_game_and_checks_checksum(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            path = directory / "server.jar"
            game = b"inner game jar"
            library = b"dependency"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("META-INF/versions.list", f"{hashlib.sha256(game).hexdigest()}\t26.3\t26.3/game.jar\n")
                archive.writestr("META-INF/versions/26.3/game.jar", game)
                archive.writestr("META-INF/libraries.list", f"{hashlib.sha256(library).hexdigest()}\tdemo:lib:1\tdemo/lib.jar\n")
                archive.writestr("META-INF/libraries/demo/lib.jar", library)
            extracted, libraries = lab.unpack_server(path, directory, "26.3")
            self.assertEqual(extracted.read_bytes(), game)
            self.assertEqual(libraries[0].read_bytes(), library)
            with self.assertRaises(ValueError):
                lab.unpack_server(path, directory, "wrong-version")

    def test_server_bundler_rejects_path_traversal(self):
        for relative in ["../../escape.jar", "/escape.jar", "..\\escape.jar"]:
            with zipfile.ZipFile(io.BytesIO(), "w") as archive:
                archive.writestr("entries", f"{'0' * 64}\tdemo\t{relative}")
                with self.assertRaisesRegex(ValueError, "Unsafe bundled path"):
                    list(lab.bundle_entries(archive, "entries"))

    def test_coverage_reports_missing_files_and_failed_method_markers(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            lab.write_json(directory / "reports" / "client-inventory.json", {
                "class_count": 3, "expected_source_files": ["demo/Example.java", "demo/Missing.java"]
            })
            sources = directory / "sources" / "client" / "demo"
            sources.mkdir(parents=True)
            (sources / "Example.java").write_text("// $VF: Couldn't be decompiled\n")
            report = lab.coverage(directory, "client")
            self.assertEqual(report["missing_source_files"], ["demo/Missing.java"])
            self.assertEqual(report["files_with_decompiler_error_markers"], ["demo/Example.java"])


class PublicationTests(unittest.TestCase):
    def test_private_paths_rejected(self):
        for path in [".local/minecraft/source.java", "decompiled/Foo.txt", "game.JAR", "capture.jfr", "source/Foo.java"]:
            self.assertTrue(check_public_tree.prohibited(path), path)
        for path in ["tools/lab.py", "crates/voxel-kernels/src/lib.rs", "Cargo.lock", "docs/analysis.md",
                     "mods/horizon/src/main/java/dev/horizonlab/HorizonLab.java",
                     "mods/horizon-lab/src/main/java/dev/horizonlab/HorizonLab.java"]:
            self.assertFalse(check_public_tree.prohibited(path), path)
        self.assertTrue(check_public_tree.prohibited("mods/horizon/src/main/java/net/minecraft/Client.java"))
        for path in ["tools/JfrSummary.java", "tests/java/JfrFixture.java"]:
            self.assertFalse(check_public_tree.prohibited(path), path)
        for path in ["tools/Minecraft.java", "tests/java/Client.java", ".local/tools/JfrSummary.java"]:
            self.assertTrue(check_public_tree.prohibited(path), path)

    def test_force_added_ignored_file_is_detected_in_index_and_history(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            real_check_output = subprocess.check_output
            def git(*args):
                return real_check_output(["git", *args], cwd=directory, stderr=subprocess.DEVNULL)
            git("init", "-b", "main")
            (directory / ".gitignore").write_text("/.local/\n")
            (directory / ".local").mkdir()
            (directory / ".local" / "artifact.txt").write_text("private fixture")
            git("add", ".gitignore")
            git("add", "-f", ".local/artifact.txt")
            with mock.patch("tools.check_public_tree.subprocess.check_output",
                            side_effect=lambda command: git(*command[1:])):
                self.assertIn(".local/artifact.txt", check_public_tree.paths_in_repository())
                git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "fixture")
                self.assertIn(".local/artifact.txt", check_public_tree.paths_in_repository(history=True))


if __name__ == "__main__":
    unittest.main()
