from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from tools import lab, vanilla_client


def native(classifier, system):
    name = f"lwjgl-3.4.3-{classifier}.jar"
    return {"name": f"org.lwjgl:lwjgl:3.4.3:{classifier}", "rules": [{"action": "allow", "os": {"name": system}}],
            "downloads": {"artifact": {"path": f"org/lwjgl/lwjgl/3.4.3/{name}", "sha1": "0", "size": 0}}}


def manifest(*classifiers):
    systems = {"linux": "linux", "macos": "osx", "windows": "windows"}
    plain = {"name": "org.lwjgl:lwjgl:3.4.3",
             "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.4.3/lwjgl-3.4.3.jar", "sha1": "0", "size": 0}}}
    return {"libraries": [plain, *(native(c, systems[c.split("-")[1]]) for c in classifiers)]}


# The LWJGL natives a 26.3 version JSON lists.
VERSION = manifest("natives-linux", "natives-macos", "natives-macos-arm64", "natives-windows", "natives-windows-arm64")


class NativesTests(unittest.TestCase):
    def test_each_platform_gets_its_own_natives(self):
        for system, architecture, expected in [
                ("windows", "AMD64", ("natives-windows.jar", ".dll")),
                ("windows", "ARM64", ("natives-windows-arm64.jar", ".dll")),
                ("linux", "x86_64", ("natives-linux.jar", ".so")),
                ("osx", "arm64", ("natives-macos-arm64.jar", ".dylib")),
                ("osx", "x86_64", ("natives-macos.jar", ".dylib"))]:
            with self.subTest(system=system, architecture=architecture):
                self.assertEqual(vanilla_client.natives_for(VERSION, system, architecture), expected)

    def test_windows_on_arm_takes_the_x64_natives_when_the_version_has_no_arm_build(self):
        version = manifest("natives-linux", "natives-windows")
        self.assertEqual(vanilla_client.natives_for(version, "windows", "ARM64"), ("natives-windows.jar", ".dll"))

    def test_a_version_without_natives_for_the_platform_is_an_error(self):
        with self.assertRaises(ValueError):
            vanilla_client.natives_for(manifest("natives-linux"), "windows", "AMD64")


class OutputTests(unittest.TestCase):
    def test_bytes_in_another_encoding_never_stop_the_reader(self):
        # cp1252's "é" then a byte no code page maps, then the line a harness waits for.
        script = r"import sys; sys.stdout.buffer.write(b'caf\xe9 \x81\n[VanillaTickScript] ready\n')"
        process = vanilla_client.spawn([sys.executable, "-c", script], None, None)
        with process.stdout, process.stdin:
            lines = list(process.stdout)
        process.wait()
        self.assertEqual(lines, ["caf\N{REPLACEMENT CHARACTER} \N{REPLACEMENT CHARACTER}\n",
                                 "[VanillaTickScript] ready\n"])

    def test_the_jvm_is_asked_for_utf8_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(game_dir=Path(tmp), assets=Path(tmp), heap="1G", name="Guest", width=1, height=1,
                             join=None, world=None, lan_port=None, backend=None)
            with mock.patch.object(vanilla_client, "classpath", return_value=[]), \
                    mock.patch.object(vanilla_client, "host", return_value=("natives-windows.jar", ".dll")), \
                    mock.patch.object(vanilla_client, "GAME", Path(tmp)):
                (Path(tmp) / "manifest.json").write_text('{"assetIndex": {"id": "1"}}')
                command = vanilla_client.command(args)
        self.assertIn("-Dstdout.encoding=UTF-8", command)
        self.assertIn("-Dstderr.encoding=UTF-8", command)


class BinaryTests(unittest.TestCase):
    def test_a_built_program_is_found_with_or_without_exe(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            (folder / "horizon-client.exe").touch()
            # Only lab's os.name: pathlib keeps this machine's path flavour.
            with mock.patch.object(lab, "os", SimpleNamespace(name="nt")):
                self.assertEqual(lab.binary(folder, "horizon-client"), folder / "horizon-client.exe")
                (folder / "horizon-client").touch()
                self.assertEqual(lab.binary(folder, "horizon-client"), folder / "horizon-client")
            with mock.patch.object(lab, "os", SimpleNamespace(name="posix")):
                self.assertEqual(lab.binary(folder, "horizon-ctl"), folder / "horizon-ctl")


if __name__ == "__main__":
    unittest.main()
