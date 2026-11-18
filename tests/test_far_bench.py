import io
import json
from pathlib import Path
import tempfile
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

# The standalone imports change sys.path and cache tools' test_* modules by bare names.
# Neither may stay to shadow unittest discovery's tests/ modules.
with patch.object(sys, 'path', sys.path.copy()), patch.dict(sys.modules):
    from tools import far_bench as bench


class FarBenchWindowTests(unittest.TestCase):
    def test_server_cpu_includes_work_before_the_last_bot_joins(self):
        clock = [1000.0]
        cpu = [20.0, 100.0]  # Cumulative far-thread and whole-server CPU before launch.

        def joined(*args):
            clock[0] += 2.0
            cpu[0] += 2.0
            cpu[1] += 3.0

        server = SimpleNamespace(process=SimpleNamespace(pid=123), wait_for=joined, stop=lambda: None)

        def launch(command, **kwargs):
            # Early requests finish while later bots join, before setup's delay ends.
            cpu[0] += 2.0
            cpu[1] += 5.0
            tile = {'planned': 1, 'served': 1, 'refused': 0, 'lost': 0, 'bytes': 10,
                    'seconds': 1.0, 'latency_ms_p50': 0, 'dump_failed': False}
            report = Path(command[command.index('--report') + 1])
            report.write_text(json.dumps({'per_bot': [{'far': tile} for _ in range(8)]}))
            return SimpleNamespace(stdout=io.StringIO('{"far_done":8}\n'), poll=lambda: 0, wait=lambda: 0)

        def delay(seconds):
            clock[0] += seconds
            cpu[1] += 1.0

        def thread(target, **kwargs):
            return SimpleNamespace(start=target)

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            out = directory / 'out'
            out.mkdir()
            with patch.object(bench, 'server', return_value=(server, 25565)), \
                    patch.object(bench, 'thread_cpu', side_effect=lambda *args: tuple(cpu)), \
                    patch.object(bench.subprocess, 'Popen', side_effect=launch), \
                    patch.object(bench.threading, 'Thread', side_effect=thread), \
                    patch.object(bench.time, 'time', side_effect=lambda: clock[0]), \
                    patch.object(bench.time, 'sleep', side_effect=delay):
                result = bench.run(directory, 'test', 8, 'together', 32, None, directory,
                                   out, False, 'horizon-bots', 90)
        self.assertEqual(result['server_cpu_seconds'], 9.0)
        self.assertEqual(result['far_cpu_seconds'], 4.0)
        self.assertEqual(result['measured_seconds'], 9.0)
        self.assertTrue(result['complete'])


if __name__ == '__main__':
    unittest.main()
