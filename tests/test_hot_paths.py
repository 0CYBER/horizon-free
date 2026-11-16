"""tools/hot_paths.py: what a published record may hold, which runs a trend compares, and what a dump counts."""
import copy
import importlib.util
import unittest
from pathlib import Path

# Loaded by path: tools/ on sys.path would shadow tests/ modules that share a name with a tools/ harness.
_spec = importlib.util.spec_from_file_location('hot_paths', Path(__file__).resolve().parents[1] / 'tools' / 'hot_paths.py')
hot_paths = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hot_paths)


def run(commit, date, value, host='alpha', checksum='0' * 16, wall=100.0):
    return {'schema': 1, 'commit': commit * 40, 'date': date, 'run': '7.1', 'host': host, 'cpu': 'cpu', 'kernel': '6',
            'rustc': 'rustc 1', 'valgrind': 'valgrind-3',
            'workloads': {'server.tick.mobs_128': {'operations': 12800, 'checksum': checksum}},
            'metrics': {'server.tick.mobs_128.instructions': [value, value, value],
                        'server.tick.mobs_128.wall_ns': [wall, wall * 1.05]},
            'notes': []}


class Record(unittest.TestCase):
    def test_a_record_keeps_known_fields_only(self):
        result = run('a', '2026-11-12T10:00:00Z', 5000.0)
        result['game_dir'] = '/home/someone/world'
        result['notes'] = ['read /home/someone/assets/secret.json',
                           'instructions and allocations not measured: no valgrind on this machine']
        kept = hot_paths.clean(result)
        self.assertNotIn('game_dir', kept)
        self.assertEqual(kept['notes'], ['instructions and allocations not measured: no valgrind on this machine'])

    def test_a_path_or_an_unknown_metric_is_refused(self):
        for change in ({'host': '/srv/work/runner'}, {'cpu': 'C:\\Users\\someone'}, {'commit': 'main'},
                       {'metrics': {'server.tick.mobs_128.assets': [1.0]}},
                       {'metrics': {'other.workload.instructions': [1.0]}},
                       {'workloads': {'../escape': {'operations': 1, 'checksum': '0' * 16}}}):
            result = {**run('a', '2026-11-12T10:00:00Z', 5000.0), **change}
            with self.assertRaises(ValueError, msg=change):
                hot_paths.clean(result)


class Trend(unittest.TestCase):
    NAME = 'server.tick.mobs_128.instructions'

    def test_a_step_needs_another_commit_and_more_than_the_noise(self):
        runs = [run('a', '2026-11-10T10:00:00Z', 5000.0), run('b', '2026-11-11T10:00:00Z', 5100.0)]
        points = hot_paths.series(runs, self.NAME)
        self.assertAlmostEqual(hot_paths.change(points, 1), 0.02)
        self.assertTrue(hot_paths.stepped(points))
        self.assertIsNone(hot_paths.change(points, 7))
        # The same move between two runs of one commit is noise, not a step.
        again = [runs[0], run('a', '2026-11-11T10:00:00Z', 5100.0)]
        points = hot_paths.series(again, self.NAME)
        self.assertFalse(hot_paths.stepped(points))
        self.assertAlmostEqual(hot_paths.noise(points), 100 / 5050)
        # Samples that lie 2% apart hide a 2% move.
        noisy = copy.deepcopy(runs)
        noisy[1]['metrics'][self.NAME] = [5050.0, 5100.0, 5150.0]
        self.assertFalse(hot_paths.stepped(hot_paths.series(noisy, self.NAME)))

    def test_a_changed_workload_starts_a_new_series(self):
        runs = [run('a', '2026-11-10T10:00:00Z', 5000.0), run('b', '2026-11-11T10:00:00Z', 9000.0, checksum='1' * 16)]
        points = hot_paths.series(runs, self.NAME)
        self.assertEqual(len(points), 1)
        self.assertFalse(hot_paths.stepped(points))

    def test_wall_time_is_compared_within_one_host_label_only(self):
        runs = [run('a', '2026-11-10T10:00:00Z', 5000.0, host='alpha'), run('b', '2026-11-11T10:00:00Z', 5000.0, host='beta'),
                run('c', '2026-11-12T10:00:00Z', 5000.0, host='beta')]
        self.assertEqual(len(hot_paths.series(runs, 'server.tick.mobs_128.wall_ns')), 2)
        self.assertEqual(len(hot_paths.series(runs, self.NAME)), 3)

    def test_another_cpu_or_toolchain_starts_a_new_series_without_a_step(self):
        for key in ('cpu', 'rustc', 'valgrind'):
            runs = [run('a', '2026-11-10T10:00:00Z', 5000.0), run('b', '2026-11-11T10:00:00Z', 5000.0),
                    {**run('c', '2026-11-12T10:00:00Z', 5600.0), key: 'another'}]
            points = hot_paths.series(runs, self.NAME)
            self.assertEqual(len(points), 1, key)
            self.assertFalse(hot_paths.stepped(points), key)


class Dump(unittest.TestCase):
    def test_a_dump_gives_the_region_its_instructions_and_its_allocator_calls(self):
        dump = '\n'.join([
            'desc: Trigger: Client Request: mod.attachment.read', 'events: Ir',
            'fn=(1) game::tick', '10 100',
            'cfn=(2) mi_malloc_aligned', 'calls=3 0', '10 60',
            'cfn=(3) game::helper', 'calls=9 0', '10 40',
            'fn=(2)', '20 30',
            # The allocator's own call into another entry point is not a second allocation.
            'cfn=(4) mi_malloc', 'calls=3 0', '20 30',
            'fn=(3)', '30 10',
            'cfn=(2)', 'calls=2 0', '30 30',
            'totals: 300', ''])
        self.assertEqual(hot_paths.parse_dump(dump), ('mod.attachment.read', 300, 5))


if __name__ == '__main__':
    unittest.main()
