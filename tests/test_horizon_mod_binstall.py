"""Placeholder handling in `cargo binstall`'s pkg-url template.

`url()` promises a KeyError for a placeholder binstall would not fill in, so a typo fails here instead
of at every mod developer's download. The placeholder regex must therefore recognise any identifier-ish
name, not only lowercase letters and hyphens.
"""

import tempfile
import unittest
from pathlib import Path

import tools.horizon_mod_binstall as binstall


def _manifest_with_pkg_url(directory, pkg_url):
    manifest = Path(directory, "Cargo.toml")
    manifest.write_text(
        '[package]\nname = "cargo-horizon-mod"\nversion = "1.2.3"\n'
        f'[package.metadata.binstall]\npkg-url = "{pkg_url}"\n'
    )
    return manifest


class PlaceholderTests(unittest.TestCase):
    def test_unknown_placeholder_with_underscore_raises_key_error(self):
        """`{target_os}` must not be silently left in the download URL."""
        with tempfile.TemporaryDirectory() as directory:
            manifest = _manifest_with_pkg_url(
                directory, "https://example.invalid/{target_os}/{name}.tar.gz"
            )
            with self.assertRaises(KeyError) as raised:
                binstall.url("aarch64-unknown-linux-gnu", manifest)
            self.assertEqual(raised.exception.args[0], "target_os")

    def test_other_identifier_like_unknown_placeholders_also_raise(self):
        for placeholder in ("{TargetOS}", "{arch2}", "{binary_ext}"):
            with self.subTest(placeholder=placeholder), tempfile.TemporaryDirectory() as directory:
                manifest = _manifest_with_pkg_url(
                    directory, f"https://example.invalid/{placeholder}/{{name}}.tar.gz"
                )
                with self.assertRaises(KeyError) as raised:
                    binstall.url("x86_64-unknown-linux-gnu", manifest)
                self.assertEqual(raised.exception.args[0], placeholder.strip("{}"))

    def test_known_placeholders_still_fill_in_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = _manifest_with_pkg_url(
                directory,
                "https://example.invalid/{ name }/{ version }/{ target }/{ binary-ext }.{ archive-format }",
            )
            self.assertEqual(
                binstall.url("x86_64-pc-windows-msvc", manifest),
                "https://example.invalid/cargo-horizon-mod/1.2.3/x86_64-pc-windows-msvc/.exe.tar.gz",
            )


if __name__ == "__main__":
    unittest.main()
