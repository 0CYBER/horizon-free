from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import test_float_log_parity as parity


class FloatLogRunnerTests(unittest.TestCase):
    def test_binary_comparison_rejects_difference_and_truncated_equal_outputs(self):
        with tempfile.TemporaryDirectory() as temp:
            reference, candidate = Path(temp)/"java.bin", Path(temp)/"rust.bin"
            reference.write_bytes(bytes.fromhex("000000003f800000"))
            candidate.write_bytes(bytes.fromhex("000000003f800001"))
            with self.assertRaisesRegex(ValueError, "float bits 3f800001"):
                parity.compare(reference, candidate, 0x3f800000, 2)
            candidate.write_bytes(reference.read_bytes())
            self.assertEqual(parity.compare(reference, candidate, 0x3f800000, 2), 2)
            with self.assertRaisesRegex(ValueError, "does not cover 3 floats"):
                parity.compare(reference, candidate, 0x3f800000, 3)

    def test_exhaustive_range_covers_all_float_bits(self):
        self.assertEqual(parity.ranges(True), [(0, 1 << 32)])
        for start, count in parity.ranges(False):
            self.assertGreaterEqual(start, 0)
            self.assertLessEqual(start + count, 1 << 32)

    def test_failed_verification_removes_old_report(self):
        with tempfile.TemporaryDirectory() as temp:
            report = Path(temp)/"rust-float-log-parity/report.json"
            report.parent.mkdir()
            report.write_text('{"old_success":true}')
            with mock.patch.object(parity.lab, "LOCAL", Path(temp)), mock.patch("sys.argv", ["parity"]), \
                mock.patch.object(parity.test_rust_parity, "verify_artifacts", side_effect=ValueError("corrupt artifact")):
                with self.assertRaisesRegex(ValueError, "corrupt artifact"):
                    parity.main()
            self.assertFalse(report.exists())
