"""The refusal fixture remains a valid package with only its layout export changed."""
import hashlib
import importlib.util
import pathlib
import subprocess
import tempfile
import tomllib
import unittest
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools/ci/mismatch_wasm_layout.py"
spec = importlib.util.spec_from_file_location("mismatch", SCRIPT)
mismatch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mismatch)


def section(kind, payload):
    return bytes([kind]) + mismatch.encoded(len(payload)) + payload


def fixture(first):
    name = b"horizon_layout_ext"
    # One imported function precedes two defined functions. The exported layout
    # function is the first defined function; the other body must stay untouched.
    return (
        b"\0asm\x01\0\0\0"
        + section(1, b"\x01\x60\x00\x01\x7e")
        + section(2, b"\x01\x03env\x01f\x00\x00")
        + section(3, b"\x02\x00\x00")
        + section(7, b"\x01" + mismatch.encoded(len(name)) + name + b"\x00\x01")
        + section(10, b"\x02" + mismatch.encoded(len(first)) + first + b"\x04\x00\x42\x07\x0b")
    )


class LayoutMismatch(unittest.TestCase):
    def test_package_integrity_survives_export_body_change(self):
        original = fixture(b"\x00\x42\x7f\x0b")  # i64.const -1
        with tempfile.TemporaryDirectory() as directory:
            source = pathlib.Path(directory) / "source.hmod"
            output = pathlib.Path(directory) / "changed.hmod"
            manifest = 'id = "layout_test"\n[wasm]\nsha512 = "' + hashlib.sha512(original).hexdigest() + '"\n'
            with zipfile.ZipFile(source, "w") as archive:
                archive.writestr("horizon-mod.toml", manifest)
                archive.writestr("mod.wasm", original)
                archive.writestr("data/unchanged.txt", "kept")
            result = subprocess.check_output(["python3", str(SCRIPT), str(source), str(output)], text=True)
            self.assertEqual(result.strip(), "layout_test")
            with zipfile.ZipFile(output) as archive:
                changed = archive.read("mod.wasm")
                self.assertEqual(changed, fixture(b"\x00\x42\x00\x0b"))
                parsed = tomllib.loads(archive.read("horizon-mod.toml").decode())
                self.assertEqual(parsed["wasm"]["sha512"], hashlib.sha512(changed).hexdigest())
                self.assertEqual(archive.read("data/unchanged.txt"), b"kept")


if __name__ == "__main__":
    unittest.main()
