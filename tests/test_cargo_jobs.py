"""Regression: --job-mib must be a positive memory budget, not zero or negative."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]

spec = importlib.util.spec_from_file_location("cargo_jobs", ROOT / "tools/ci/cargo_jobs.py")
cargo_jobs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo_jobs)
GIB = 1 << 30


class CargoJobsValidationTests(unittest.TestCase):
    def test_non_positive_job_mib_is_rejected(self):
        for value in (0, -1):
            with self.subTest(job_mib=value):
                with self.assertRaises(ValueError):
                    cargo_jobs.jobs(8, 16 * GIB, value)

    def test_positive_job_mib_is_unchanged(self):
        # The normal ValueError free path must keep its old answers.
        self.assertEqual(cargo_jobs.jobs(8, 16 * GIB, 4), 8)
        self.assertEqual(cargo_jobs.jobs(8, 16 * GIB, 3072), 5)

    def test_cli_rejects_non_positive_job_mib_with_an_argument_error(self):
        for value in ("0", "-1"):
            with self.subTest(job_mib=value):
                result = subprocess.run(
                    [sys.executable, ROOT / "tools/ci/cargo_jobs.py", "--job-mib", value],
                    capture_output=True, text=True, check=False,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("--job-mib must be positive", result.stderr)
                self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
