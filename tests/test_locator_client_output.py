"""The one-player harness client's acknowledgment deadline and its compared transcript
(tools/test_locator_bar_parity.py)."""
import io
from pathlib import Path
import tempfile
import time
import unittest

from tools import test_locator_bar_parity as locator


class _Process:
    """A client whose stdout is `lines`, each after `pause` seconds."""

    def __init__(self, lines, pause=0.0):
        self.stdout = self._lines(lines, pause)

    @staticmethod
    def _lines(lines, pause):
        for line in lines:
            time.sleep(pause)
            yield line


class _SlowLog(io.StringIO):
    """A log that takes `pause` seconds a line, so a backlog outlasts a deadline."""

    def __init__(self, pause):
        super().__init__()
        self.pause = pause

    def write(self, text):
        time.sleep(self.pause)
        return super().write(text)


class Deadline(unittest.TestCase):
    def test_a_client_that_keeps_printing_without_acknowledging_fails_at_the_deadline(self):
        # Printed faster than it is logged: the queue is never empty, so only the deadline can end the wait.
        output = locator.ClientOutput(_Process(['noise\n'] * 5_000), _SlowLog(0.001), 'side')
        started = time.monotonic()
        with self.assertRaisesRegex(AssertionError, "no 'synced Alice'"):
            output.expect('synced Alice', 0.3)
        self.assertLess(time.monotonic() - started, 1.5)

    def test_an_acknowledgment_behind_a_backlog_past_the_deadline_fails(self):
        output = locator.ClientOutput(_Process(['noise\n'] * 100 + ['synced Alice\n']), _SlowLog(0.01), 'side')
        time.sleep(0.1)  # the whole backlog and the acknowledgment are queued
        with self.assertRaisesRegex(AssertionError, "no 'synced Alice'"):
            output.expect('synced Alice', 0.3)

    def test_an_acknowledgment_in_time_passes(self):
        output = locator.ClientOutput(_Process(['noise\n', 'synced Alice\n']), io.StringIO(), 'side')
        output.expect('synced Alice', 5)
        self.assertEqual(output.log.getvalue(), 'noise\n')


class Snapshot(unittest.TestCase):
    def test_the_snapshot_is_the_transcript_kept_and_the_later_one_moves_aside(self):
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / 'horizon.txt'
            out.write_text('Alice step=free-near chat ok\nAlice step=end disconnected\n', encoding='utf-8')
            lines = locator.keep_snapshot(out, 'Alice step=free-near chat ok\n')
            self.assertEqual(lines, ['Alice step=free-near chat ok'])
            self.assertEqual(out.read_text(encoding='utf-8'), 'Alice step=free-near chat ok\n')
            self.assertIn('disconnected', (Path(folder) / 'horizon-end.txt').read_text(encoding='utf-8'))


if __name__ == '__main__':
    unittest.main()
