"""A malformed --time-limits pair is a clean argparse error, never a raw ValueError."""
import contextlib
import io
import sys
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci"))
import run_test_binaries as runner  # noqa: E402


class TimeLimitsArgumentTests(unittest.TestCase):
    def assert_bad_limits(self, value):
        with contextlib.redirect_stderr(io.StringIO()) as errors, \
                mock.patch.object(runner.subprocess, "run",
                                  side_effect=AssertionError("no subprocess should start")):
            with self.assertRaises(SystemExit) as raised:
                runner.main(["--jobs", "1", "--time-limits", value, "--", "cargo", "test"])
        self.assertEqual(raised.exception.code, 2)
        self.assertIn("--time-limits", errors.getvalue())

    def test_single_value_is_a_clean_argparse_error(self):
        # "2" has no comma, so partition() used to give float("") -> ValueError with a traceback.
        self.assert_bad_limits("2")

    def test_other_malformed_pairs_are_rejected(self):
        for value in ("", "2,", ",5", "two,five", "2,x", "-1,2", "nan,5"):
            with self.subTest(value=value):
                self.assert_bad_limits(value)


if __name__ == "__main__":
    unittest.main()
