import unittest

import re

import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from types import SimpleNamespace

from tools.server_bench import drain, parse_args, spread, spread_points
from tools.server_bench import (DONE, PERF, PUMPKIN_READY, RUN_RECORD, TICK_AVERAGE, TICK_PERCENTILES, finish_profile,
                                pumpkin_config, report_line, start_profile, work)


def run(server, bots, sent, new, waited=0.0, behaviour="mixed"):
    return {"server": server, "behaviour": behaviour, "bots": bots, "chunks_per_second": sent,
            "unique_per_second": new, "waited_ticks_per_second": waited}


class ServerBenchWorkTests(unittest.TestCase):
    def test_shares_of_the_busiest_server(self):
        [row] = work([run("horizon", 10, 98.2, 36.5), run("vanilla", 10, 24.3, 19.1, 12.0)])
        self.assertFalse(row["equal"])
        self.assertEqual(row["servers"]["horizon"]["chunks_per_second"], {"value": 98.2, "share": 1.0})
        self.assertEqual(row["servers"]["vanilla"]["chunks_per_second"]["share"], 0.247)
        self.assertEqual(row["servers"]["vanilla"]["waited_ticks_per_second"], 12.0)

    def test_equal_within_ten_percent_or_a_chunk(self):
        [row] = work([run("horizon", 25, 236, 28.9), run("vanilla", 25, 252, 28.9)])
        self.assertTrue(row["equal"])
        # One bot: 0.3 against 1.1 new chunks a second is within a chunk.
        [row] = work([run("horizon", 1, 9.9, 0.3), run("vanilla", 1, 9.9, 1.1)])
        self.assertTrue(row["equal"])

    def test_only_counts_run_on_several_servers(self):
        failed = dict(run("vanilla", 50, 0, 0), error="watchdog")
        self.assertEqual(work([run("horizon", 50, 450, 10), failed, run("horizon", 1, 9, 0.3)]), [])

    def test_each_round_is_compared_on_its_own(self):
        rows = work([dict(run("horizon", 20, 236, 111), round=0), dict(run("pumpkin", 20, 190, 69), round=0),
                     dict(run("horizon", 20, 237, 112), round=1), dict(run("pumpkin", 20, 237, 112), round=1)])
        self.assertEqual([(r["round"], r["equal"]) for r in rows], [(0, False), (1, True)])


class ServerBenchLogTests(unittest.TestCase):
    def test_every_server_says_it_is_up_the_same_way(self):
        for line in ['[05:16:10] [Server thread/INFO]: Done (9.762s)! For help, type "help"',
                     '[02:53:58 INFO]: Done (29.649s)! For help, type "help"']:
            self.assertTrue(re.search(DONE, line), line)
        self.assertFalse(re.search(DONE, '[02:53:58 INFO]: Preparing spawn area: Done (12%)'))

    def test_paper_answers_tick_query_as_vanilla_does(self):
        # Paper 26.3 build 205, from a smoke run of this tool.
        self.assertEqual(TICK_AVERAGE.search("Average time per tick: 14.4ms (Target: 50.0ms)").group(1), "14.4")
        self.assertEqual(TICK_PERCENTILES.search(
            "[02:54:10 INFO]: System chat: Percentiles: P50: 1.0ms P95: 34.6ms P99: 764.4ms. Sample: 100").groups(),
            ("1.0", "34.6", "764.4"))
    def test_pumpkin_is_up_once_it_listens(self):
        line = "16:39:19  INFO Server is now running. Connect using port: Java Edition: 127.0.0.1:39811"
        self.assertTrue(re.search(PUMPKIN_READY, line))


# A configuration in the shape Pumpkin writes (made up here: the keys this tool sets).
PUMPKIN_DEFAULTS = '''seed = "17"
default_gamemode = "Survival"
spawn_protection = 16

[logging]
color = true

[networking.query]
enabled = true
address = "0.0.0.0:25565"

[networking.java]
enabled = true
address = "0.0.0.0:25565"
encryption = true
online_mode = true
view_distance = 16
simulation_distance = 10

[networking.bedrock]
enabled = true
address = "0.0.0.0:19132"
online_mode = true
view_distance = 16

[plugins]
enabled = true

[telemetry]
enabled = true
endpoint = "https://example.org/heartbeat"
'''


class PumpkinConfigTests(unittest.TestCase):
    def test_the_benchmark_settings_on_loopback_with_the_extras_off(self):
        text = pumpkin_config(PUMPKIN_DEFAULTS, 27001, SimpleNamespace(seed=12345))
        self.assertIn('seed = "12345"', text)
        self.assertIn('default_gamemode = "Creative"', text)
        self.assertIn('spawn_protection = 0', text)
        java = text.split('[networking.java]')[1].split('[')[0]
        for line in ('address = "127.0.0.1:27001"', 'encryption = false', 'online_mode = false',
                     'view_distance = 10', 'simulation_distance = 10'):
            self.assertIn(line, java)
        self.assertIn('[networking.query]\nenabled = false\naddress = "127.0.0.1:27002"', text)
        self.assertIn('[networking.bedrock]\nenabled = false', text)
        self.assertIn('[plugins]\nenabled = false', text)
        self.assertIn('[telemetry]\nenabled = false\nendpoint = ""', text)
        self.assertNotIn('0.0.0.0:25565', text)
        # Only its own section's keys change: Bedrock keeps its (it is off).
        self.assertIn('address = "0.0.0.0:19132"\nonline_mode = true', text)

    def test_settings_absent_from_the_template_are_inserted(self):
        # A trimmed template lacks encryption, max_players and simulation_distance. Keeping them out would
        # leave Pumpkin's upstream defaults in place, so all declared values must still land in the output.
        minimal = '''seed = "old"
white_list = true

[networking.java]
address = "0.0.0.0:25565"
online_mode = true
view_distance = 16

[telemetry]
enabled = true
'''
        text = pumpkin_config(minimal, 25599, SimpleNamespace(seed=12345))
        config = tomllib.loads(text)  # Duplicate inserted keys also make parsing fail.
        java = config['networking']['java']
        self.assertEqual(config['seed'], '12345')
        self.assertFalse(config['white_list'])
        for key, value in {'address': '127.0.0.1:25599', 'online_mode': False, 'encryption': False,
                           'max_players': 200, 'view_distance': 10, 'simulation_distance': 10}.items():
            self.assertEqual(java[key], value)
        self.assertEqual(config['telemetry'], {'enabled': False, 'endpoint': ''})

    def test_a_section_missing_from_the_template_is_created(self):
        # No [networking.query] or [telemetry] at all: the whole section must be added with every key.
        minimal = '''seed = "12345"

[networking.java]
address = "0.0.0.0:25565"
'''
        config = tomllib.loads(pumpkin_config(minimal, 25599, SimpleNamespace(seed=12345)))
        self.assertEqual(config['networking']['java']['address'], '127.0.0.1:25599')
        self.assertEqual(config['networking']['query'], {'enabled': False, 'address': '127.0.0.1:25600'})
        self.assertEqual(config['telemetry'], {'enabled': False, 'endpoint': ''})
        self.assertFalse(config['plugins']['enabled'])
        self.assertFalse(config['networking']['java']['authentication']['enabled'])

    def test_empty_sections_keep_the_next_header_on_its_own_line(self):
        minimal = 'seed = "0"\n\n[networking.java]\n[telemetry]\n'
        config = tomllib.loads(pumpkin_config(minimal, 25599, SimpleNamespace(seed=12345)))
        self.assertEqual(config['seed'], '12345')
        self.assertEqual(config['networking']['java']['address'], '127.0.0.1:25599')
        self.assertEqual(config['networking']['java']['simulation_distance'], 10)
        self.assertEqual(config['telemetry'], {'enabled': False, 'endpoint': ''})

    def test_existing_keys_with_other_legal_spacing_are_replaced(self):
        minimal = 'seed\t=\t"0"\n[networking.java]\n\tmax_players=20\n  online_mode\t=\ttrue\n'
        config = tomllib.loads(pumpkin_config(minimal, 25599, SimpleNamespace(seed=12345)))
        self.assertEqual(config['seed'], '12345')
        self.assertEqual(config['networking']['java']['max_players'], 200)
        self.assertFalse(config['networking']['java']['online_mode'])

    def test_a_leading_table_keeps_inserted_root_settings_at_the_root(self):
        minimal = '[networking.java]\naddress = "0.0.0.0:25565"\n'
        config = tomllib.loads(pumpkin_config(minimal, 25599, SimpleNamespace(seed=12345)))
        self.assertEqual(config['seed'], '12345')
        self.assertNotIn('seed', config['networking']['java'])
        self.assertEqual(config['networking']['java']['address'], '127.0.0.1:25599')

    def test_commented_table_headers_are_not_appended_again(self):
        minimal = ('seed = "0"\n[networking.java] # Java listener\naddress = "0.0.0.0:25565"\n'
                   '[telemetry]\t# Telemetry\nenabled = true\n')
        config = tomllib.loads(pumpkin_config(minimal, 25599, SimpleNamespace(seed=12345)))
        self.assertEqual(config['networking']['java']['address'], '127.0.0.1:25599')
        self.assertEqual(config['telemetry'], {'enabled': False, 'endpoint': ''})


class FakeServer:
    """The console and the log of a server that prints what Horizon prints."""

    def __init__(self, lines=()):
        self.typed = []
        self.lines = list(lines)

    def console(self, line):
        self.typed.append(line)

    def since(self, n):
        return self.lines[n:]

    def wait_for(self, pattern, timeout):
        if not any(re.search(pattern, l) for l in self.lines):
            raise RuntimeError(f'no {pattern!r} within {timeout} s')


SAVED_LINE = ('[10:02:11] [Server thread/INFO]: Horizon: Saved reports/profile-20261108-100211Z.hzperf '
              '(the file stays on your computer)')


class ServerBenchProfileTests(unittest.TestCase):
    def test_the_report_is_asked_for_the_measure_window_with_the_given_flags(self):
        for profile, seconds, line in [('', 300, 'horizon-report 300'),
                                       ('--no-samples', 120, 'horizon-report 120 --no-samples'),
                                       # The game ends a report at 600 s.
                                       ('', 900, 'horizon-report 600')]:
            server = FakeServer()
            args = SimpleNamespace(profile=profile, seconds=seconds)
            self.assertTrue(start_profile(server, 'horizon', args))
            self.assertEqual(server.typed, [line])
            self.assertEqual(report_line(seconds, profile), line)

    def test_nothing_is_recorded_without_the_flag_or_on_another_server(self):
        server = FakeServer()
        self.assertFalse(start_profile(server, 'horizon', SimpleNamespace(profile=None, seconds=300)))
        for kind in ('vanilla', 'paper', 'fabric', 'pumpkin'):
            self.assertFalse(start_profile(server, kind, SimpleNamespace(profile='', seconds=300)))
        self.assertEqual(server.typed, [])

    def test_the_run_record_is_attached_to_the_capture_the_server_saved(self):
        record = {'server': 'horizon', 'bots': 25, 'mspt': {'average': 12.4, 'p95': 20.1}}
        calls = []

        def attach(argv, **kwargs):
            body = Path(argv[argv.index('--body') + 1])
            calls.append((argv, json.loads(body.read_text()), kwargs))
            return subprocess.CompletedProcess(argv, 0)

        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            result = dict(record, bots_failed=[{'error': 'connect to 10.0.0.5:25599 refused'}])
            finish_profile(FakeServer(['starting', SAVED_LINE]), directory, result, run=attach)
        [(argv, body, kwargs)] = calls
        capture = directory / 'reports/profile-20261108-100211Z.hzperf'
        self.assertEqual(argv[:3], [PERF, 'attach', capture])
        self.assertEqual(argv[3:5], ['--name', RUN_RECORD])
        self.assertEqual(body, record)
        self.assertTrue(kwargs['check'])
        self.assertEqual(result['profile'], str(capture))

    def test_a_server_that_never_saves_leaves_the_run_standing_with_the_reason(self):
        calls = []
        result = {'server': 'horizon'}
        with tempfile.TemporaryDirectory() as folder:
            finish_profile(FakeServer(['starting']), Path(folder), result, run=lambda *a, **k: calls.append(a))
        self.assertEqual(calls, [])
        self.assertNotIn('profile', result)
        self.assertIn('Saved', result['profile_error'])

    def test_a_failing_attach_is_noted_and_the_run_stands(self):
        def fails(argv, **kwargs):
            raise subprocess.CalledProcessError(1, argv)

        result = {'server': 'horizon'}
        with tempfile.TemporaryDirectory() as folder:
            finish_profile(FakeServer([SAVED_LINE]), Path(folder), result, run=fails)
        self.assertNotIn('profile', result)
        self.assertIn('profile_error', result)


class SpreadTests(unittest.TestCase):
    def test_spread_is_off_unless_asked_and_takes_the_side_of_the_square(self):
        self.assertEqual(parse_args([]).spread, 0)
        args = parse_args(['--spread', '2000', '--seed', '7'])
        self.assertEqual((args.spread, args.seed), (2000, 7))
        with self.assertRaises(SystemExit):
            parse_args(['--spread', '-5'])

    def test_the_points_repeat_with_the_seed_and_stay_in_the_square(self):
        points = spread_points(200, 2000, 12345)
        self.assertEqual(len(points), 200)
        self.assertEqual(points, spread_points(200, 2000, 12345))
        self.assertNotEqual(points, spread_points(200, 2000, 12346))
        self.assertTrue(all(-1000 <= dx <= 1000 and -1000 <= dz <= 1000 for dx, dz in points))
        # Spread over the square, not bunched near the spawn: every quarter of it gets bots.
        self.assertEqual({(dx > 0, dz > 0) for dx, dz in points}, {(False, False), (False, True), (True, False),
                                                                   (True, True)})

    def test_every_bot_goes_to_its_point_around_the_first_spawn_at_its_flying_height(self):
        lines = ['{"spawn":[8.5,71.0,-3.5]}\n', '{"t":3,"online":3,"failed":0,"chunks":0}\n']
        server = FakeServer()
        self.assertEqual(spread(server, lines, 3, 100, 12345, wait=0, height=24), 3)
        self.assertEqual(len(server.typed), 3)
        for i, line in enumerate(server.typed):
            tp, name, x, y, z = line.split()
            self.assertEqual((tp, name, float(y)), ('tp', f'Bot{i}', 95.0))
            self.assertLessEqual(abs(float(x) - 8.5), 50)
            self.assertLessEqual(abs(float(z) + 3.5), 50)

    def test_no_spawned_bot_fails_the_run(self):
        server = FakeServer()
        with self.assertRaises(RuntimeError):
            spread(server, ['{"t":1,"online":0,"failed":2}\n'], 2, 100, 12345, wait=0)
        self.assertEqual(server.typed, [])


class BotStderrTests(unittest.TestCase):
    def test_a_chatty_process_never_blocks_and_its_last_lines_stay(self):
        # Far more than a pipe holds: unread, the child blocks on its stderr and never exits.
        code = "import sys\nfor i in range(20000):\n    print(f'line {i}', file=sys.stderr)\n"
        process = subprocess.Popen([sys.executable, '-I', '-c', code], stdout=subprocess.DEVNULL,
                                   stderr=subprocess.PIPE, text=True)
        self.addCleanup(process.stderr.close)
        self.addCleanup(process.kill)
        tail, reader = drain(process.stderr, keep=3)
        self.assertEqual(process.wait(timeout=30), 0)
        reader.join(timeout=30)
        self.assertEqual(list(tail), ['line 19997', 'line 19998', 'line 19999'])


if __name__ == "__main__":
    unittest.main()
