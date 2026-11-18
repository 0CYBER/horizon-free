"""Regression tests for the data-pack upgrade fidelity harness's log scan."""
import unittest

from tools import datapack_upgrade_fidelity as fidelity


class ErrorsPatternTests(unittest.TestCase):
    def test_benign_chat_is_not_a_pack_error(self):
        # A player chat echo merely contains the words "Missing"/"No key"; it is not a
        # data-pack or registry diagnostic, so it must never become a fatal problem.
        for line in ("<Steve> Missing key", "<Alex> No key for that",
                     "[20:15:30] [Server thread/INFO]: <Steve> Missing key"):
            with self.subTest(line=line):
                self.assertIsNone(fidelity.ERRORS.search(line))

    def test_real_pack_errors_still_match(self):
        for line in ("[Server] Failed to load datapacks",
                     "Missing key minecraft:foo",
                     "[20:15:30] [Server thread/ERROR]: Missing key minecraft:foo",
                     "[20:15:30] [Server thread/ERROR]: No key type in registry",
                     "Unknown registry key minecraft:missing",
                     "Errors in registry minecraft:predicate"):
            with self.subTest(line=line):
                self.assertIsNotNone(fidelity.ERRORS.search(line))


if __name__ == "__main__":
    unittest.main()
