from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import test_tick_parity as parity


class TickParityRunnerTests(unittest.TestCase):
    def test_failed_reference_verification_removes_old_success(self):
        with tempfile.TemporaryDirectory() as temp:
            local = Path(temp)
            report = local / "rust-tick-parity/report.json"
            report.parent.mkdir()
            report.write_text('{"previous_success":true}')
            with mock.patch.object(parity.lab, "LOCAL", local), mock.patch("sys.argv", ["tick-parity"]), \
                 mock.patch.object(parity.test_rust_parity, "verify_artifacts", side_effect=ValueError("corrupt reference")):
                with self.assertRaisesRegex(ValueError, "corrupt reference"):
                    parity.main()
            self.assertFalse(report.exists())

    def test_corpus_repeats_and_covers_callback_actions_and_limits(self):
        commands = list(parity.corpus())
        self.assertEqual(commands, list(parity.corpus()))
        tick_commands = [line.split() for line in commands if line.startswith("tick ")]
        self.assertEqual({int(words[3]) for words in tick_commands}, set(range(6)))
        self.assertTrue({-1, 0, 1, 100000} <= {int(words[2]) for words in tick_commands})
        self.assertTrue({"load", "reload", "copy", "clear", "enable", "restore", "save", "query"} <= {line.split()[0] for line in commands})


if __name__ == "__main__":
    unittest.main()
