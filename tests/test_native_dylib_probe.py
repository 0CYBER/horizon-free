"""Pre-load exact-build refusal tests; no SDK build required."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

TOOLS = Path(__file__).resolve().parents[1] / "tools"
sys.path.append(str(TOOLS))
spec = importlib.util.spec_from_file_location("native_dylib_probe", TOOLS / "native_dylib_probe.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class PreflightTests(unittest.TestCase):
    def test_exact_compiler_and_artifact_required(self):
        compiler = "rustc example\nrelease: 1.99.1\nhost: test\n"
        with tempfile.TemporaryDirectory() as directory:
            sdk = Path(directory)
            (sdk / "artifact").write_bytes(b"original")
            (sdk / "horizon-prebuilt.json").write_text(json.dumps({
                "rustc": compiler, "files": {"artifact": hashlib.sha256(b"original").hexdigest()}
            }))
            probe.preflight(sdk, compiler)
            with self.assertRaisesRegex(ValueError, "full rustc"):
                probe.preflight(sdk, compiler.replace("example", "other commit"))
            (sdk / "artifact").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "artifact mismatch"):
                probe.preflight(sdk, compiler)

    def test_unpinned_compiler_refused_even_when_sdk_agrees(self):
        with tempfile.TemporaryDirectory() as directory:
            sdk = Path(directory)
            compiler = "release: 1.99.2\n"
            (sdk / "horizon-prebuilt.json").write_text(json.dumps({"rustc": compiler, "files": {}}))
            with self.assertRaisesRegex(ValueError, "pinned"):
                probe.preflight(sdk, compiler)


ALLOC = "_RNvCs5fGHjzWXRav_7___rustc12___rust_alloc"
BACKTRACE = "_RINvNtNtCs9q_3std3sys9backtrace28___rust_begin_short_backtraceINtNtCsi_5alloc3vec3VecEE"


class WindowsAllocatorTests(unittest.TestCase):
    def verdict(self, tables):
        original = probe.pe_symbols
        probe.pe_symbols = lambda image: tables[image.name]
        try:
            with tempfile.TemporaryDirectory() as directory:
                return probe.windows_allocator([Path(name) for name in tables], None, Path(directory) / "rows.json")
        finally:
            probe.pe_symbols = original

    def test_game_without_allocator_and_imports_from_std_pass(self):
        rows = self.verdict({
            "horizon-client.exe": ([], {"std.dll": [ALLOC]}),
            "horizon_client.dll": ([BACKTRACE], {"std.dll": [ALLOC]}),
            "std.dll": ([ALLOC], {}),
        })
        self.assertEqual(rows["horizon_client.dll"]["exports"], [])

    def test_second_exporter_refused(self):
        with self.assertRaisesRegex(ValueError, "one image exporting"):
            self.verdict({"horizon-client.exe": ([], {"std.dll": [ALLOC]}),
                          "horizon_client.dll": ([ALLOC], {}), "std.dll": ([ALLOC], {})})

    def test_import_from_a_non_exporter_refused(self):
        with self.assertRaisesRegex(ValueError, "other than std.dll"):
            self.verdict({"horizon-client.exe": ([], {"horizon_client.dll": [ALLOC]}),
                          "horizon_client.dll": ([], {}), "std.dll": ([ALLOC], {})})
