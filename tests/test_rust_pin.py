"""The launcher and wasm mod builders share an exact Rust release."""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class RustPinTests(unittest.TestCase):
    def test_the_pin_is_one_exact_release(self):
        self.assertRegex((ROOT / "tools/ci/rust-version").read_text(), r"^\d+\.\d+\.\d+\n?$")

    def test_the_launcher_and_wasm_toolchains_read_the_pin(self):
        for path, include in (("launcher/src/rust_toolchain.rs", '"../../tools/ci/rust-version"'),
                              ("crates/cargo-horizon-mod/src/wasm.rs", '"../../../tools/ci/rust-version"')):
            with self.subTest(path=path):
                self.assertIn(f"include_str!({include}).trim_ascii()", (ROOT / path).read_text())


if __name__ == "__main__":
    unittest.main()
