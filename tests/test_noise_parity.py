from collections import Counter
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import test_noise_parity as parity


class NoiseRunnerTests(unittest.TestCase):
    def test_failed_reference_verification_removes_previous_success(self):
        with tempfile.TemporaryDirectory() as temp:
            report = Path(temp) / "rust-noise-parity/report.json"
            report.parent.mkdir()
            report.write_text('{"previous_success":true}')
            with mock.patch.object(parity.lab, "LOCAL", Path(temp)), mock.patch("sys.argv", ["parity"]), \
                 mock.patch.object(parity.test_rust_parity, "verify_artifacts", side_effect=ValueError("corrupt artifact")):
                with self.assertRaisesRegex(ValueError, "corrupt artifact"):
                    parity.main()
            self.assertFalse(report.exists())

    def test_corpus_replays_both_streams_offsets_and_accumulation_modes(self):
        commands = list(parity.corpus())
        self.assertEqual(commands, list(parity.corpus()))
        counts = Counter(line.split()[0] for line in commands)
        self.assertGreater(counts["query"], 190000)
        self.assertEqual(counts["local"], 38400)
        creates = [line.split() for line in commands if line.startswith("create ")]
        self.assertEqual({(line[1], line[2], line[4]) for line in creates},
                         {(kind, stream, skip) for stream in ["legacy", "xoroshiro"]
                          for kind, skip in [("perlin", "0"), ("simplex", "0"), ("simplex", "1"), ("smeared", "0")]})
        self.assertEqual({line.split()[-1] for line in commands if line.startswith("volume ")}, {"0", "1", "2", "3"})
        wrapped = {line.split()[1] for line in commands if line.startswith("wrap ")}
        self.assertTrue({"8000000000000000", "7ff0000000000000", "fff0000000000000", "7ff8000000000000", "0000000000000001"} <= wrapped)
        normal = [line.split() for line in commands if line.startswith("normal ")]
        self.assertEqual({line[1] for line in normal}, {"legacy", "xoroshiro"})
        self.assertEqual({line[3] for line in normal}, {"0", "1", "2", "3"})
        self.assertEqual({line[6] for line in normal}, {"0", "1", "2"})
        self.assertIn("stack 0 ", commands)


if __name__ == "__main__":
    unittest.main()
