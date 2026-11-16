import contextlib
import io
import subprocess
import unittest
from unittest import mock

from tools import check_glibc


class GlibcBaselineTests(unittest.TestCase):
    def test_ubuntu_2204_and_older_requirements_pass(self):
        report = """Version needs section '.gnu.version_r' contains 1 entry:
  File: libc.so.6 Cnt: 3
  Name: GLIBC_2.2.5 Flags: none Version: 4
  Name: GLIBC_2.9 Flags: none Version: 3
  Name: GLIBC_2.35 Flags: none Version: 2
"""
        self.assertEqual(check_glibc.incompatible_versions(report), [])
        self.assertEqual(check_glibc.incompatible_versions("No version information found in this file."), [])

    def test_newer_numeric_and_abi_requirements_fail(self):
        report = "Version needs section '.gnu.version_r' contains 1 entry:\n" + "\n".join(f"Name: {version} Flags: none" for version in [
            "GLIBC_2.36", "GLIBC_2.38", "GLIBC_2.100", "GLIBC_ABI_DT_RELR", "GLIBC_PRIVATE",
            "GLIBCXX_3.4.32", "CXXABI_1.3.13",
        ])
        self.assertEqual(set(check_glibc.incompatible_versions(report)), {
            "GLIBC_2.36", "GLIBC_2.38", "GLIBC_2.100", "GLIBC_ABI_DT_RELR", "GLIBC_PRIVATE",
        })

    def test_exported_version_definitions_are_not_runtime_requirements(self):
        report = """Version definition section '.gnu.version_d' contains 1 entry:
  Name: GLIBC_2.99
Version needs section '.gnu.version_r' contains 1 entry:
  Name: GLIBC_2.34 Flags: none
"""
        self.assertEqual(check_glibc.incompatible_versions(report), [])

    def test_cli_rejects_one_incompatible_binary_without_hiding_its_name(self):
        def readelf(arguments, **kwargs):
            self.assertEqual(kwargs["env"]["LC_ALL"], "C", "readelf's field names must not be localized")
            required = "GLIBC_2.35" if arguments[-1] == "client" else "GLIBC_2.38"
            return subprocess.CompletedProcess(arguments, 0, f"Version needs section '.gnu.version_r':\nName: {required} Flags: none", "")
        errors = io.StringIO()
        with mock.patch("sys.argv", ["check_glibc.py", "client", "server"]), \
                mock.patch.object(check_glibc.shutil, "which", return_value="readelf"), \
                mock.patch.object(check_glibc.subprocess, "run", side_effect=readelf), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(errors):
            self.assertEqual(check_glibc.main(), 1)
        self.assertIn("server: requires GLIBC_2.38", errors.getvalue())
        self.assertIn("glibc 2.35 (Ubuntu 22.04)", errors.getvalue())
        self.assertNotIn("client:", errors.getvalue())

    def test_unreadable_elf_is_not_a_passing_baseline(self):
        result = subprocess.CompletedProcess([], 1, "", "readelf: not an ELF file\n")
        errors = io.StringIO()
        with mock.patch("sys.argv", ["check_glibc.py", "bad"]), \
                mock.patch.object(check_glibc.shutil, "which", return_value="readelf"), \
                mock.patch.object(check_glibc.subprocess, "run", return_value=result), \
                contextlib.redirect_stderr(errors):
            self.assertEqual(check_glibc.main(), 1)
        self.assertIn("not an ELF file", errors.getvalue())


if __name__ == "__main__":
    unittest.main()
