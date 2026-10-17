#!/usr/bin/env python3
"""Server benchmark: the same bots on Horizon's server and on vanilla 26.3's (docs/server.md, section 9).

Every run starts a fresh server from the same server.properties (seed, view and simulation distance 10,
offline, creative so the bots can fly), waits for `Done`, lets `horizon-bots` join one a second, warms up,
then measures for a while:

- MSPT (average, P50, P95, P99) from each server's own `/tick query`, asked every 5 seconds on the console:
  vanilla's ServerTickRateManager numbers, which Horizon reproduces, so both measure themselves the same way;
- RSS and CPU of the server's whole process tree, once a second;
- chunks the bots receive per second (new terrain while the `explore` bots fly outwards);
- the start time to `Done`, and every bot that failed to join or was dropped.

    python3 tools/server_bench.py --servers horizon,vanilla --bots 1,10,25 --seconds 120

The vanilla side runs the pinned server jar from tools/lab.py prepare (your own copy). It needs Java 25. The
Horizon side runs target/release/horizon-server with the engine beside it; build both first:
    cargo build --release -p horizon-client --bin horizon-client -p horizon-server -p horizon-bots
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import statistics
import subprocess
import sys
import threading
import time

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

ROOT = lab.ROOT
EXE = '.exe' if os.name == 'nt' else ''
# Where horizon-server and horizon-bots are (the engine is found beside horizon-server or on the PATH).
BIN = Path(os.environ.get('HORIZON_BENCH_BIN', ROOT / 'target/release'))

PROPERTIES = {
    'level-seed': '12345',
    'view-distance': '10',
    'simulation-distance': '10',
    'online-mode': 'false',
    'white-list': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'spawn-protection': '0',
    'max-players': '200',
    'difficulty': 'easy',
    'sync-chunk-writes': 'true',
}


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def cpu_seconds(text):
    """ps's cumulative CPU time: [[DD-]HH:]MM:SS[.ss]."""
    days, _, rest = text.rpartition('-')
    parts = [float(p) for p in rest.split(':')]
    seconds = 0.0
    for p in parts:
        seconds = seconds * 60 + p
    return seconds + (float(days) * 86400 if days else 0)


def tree_usage(root):
    """RSS (bytes) and CPU seconds of a process and all its descendants."""
    out = subprocess.run(['ps', '-A', '-o', 'pid=,ppid=,rss=,time='], capture_output=True, text=True).stdout
    rows = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 4:
            rows[int(parts[0])] = (int(parts[1]), int(parts[2]) * 1024, cpu_seconds(parts[3]))
    tree, frontier = {root}, [root]
    while frontier:
        parent = frontier.pop()
        for pid, (ppid, _, _) in rows.items():
            if ppid == parent and pid not in tree:
                tree.add(pid)
                frontier.append(pid)
    rss = sum(rows[p][1] for p in tree if p in rows)
    cpu = sum(rows[p][2] for p in tree if p in rows)
    return rss, cpu


class Server:
    """A server with its console on stdin and its output collected."""

    def __init__(self, command, directory, log_path):
        self.lines = []
        self.lock = threading.Lock()
        self.log = log_path.open('w', encoding='utf-8')
        self.process = subprocess.Popen([str(c) for c in command], cwd=directory, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                        encoding='utf-8', errors='replace',
                                        env=dict(os.environ, HORIZON_NO_AUDIO='1'))
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.process.stdout:
            with self.lock:
                self.lines.append(line.rstrip('\n'))
            self.log.write(line)
            self.log.flush()

    def count(self):
        with self.lock:
            return len(self.lines)

    def since(self, n):
        with self.lock:
            return self.lines[n:]

    def wait_for(self, pattern, timeout):
        regex = re.compile(pattern)
        deadline = time.time() + timeout
        while time.time() < deadline:
            if any(regex.search(l) for l in self.since(0)):
                return
            if self.process.poll() is not None:
                raise RuntimeError('the server exited:\n' + '\n'.join(self.since(0)[-20:]))
            time.sleep(0.1)
        raise RuntimeError(f'no {pattern!r} within {timeout} s')

    def console(self, line):
        self.process.stdin.write(line + '\n')
        self.process.stdin.flush()

    def stop(self):
        if self.process.poll() is None:
            try:
                self.console('stop')
                self.process.wait(timeout=180)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait()
        self.log.close()


TICK_AVERAGE = re.compile(r'Average time per tick: ([0-9.]+) ?ms')
TICK_PERCENTILES = re.compile(r'P50: ([0-9.]+) ?ms P95: ([0-9.]+) ?ms P99: ([0-9.]+) ?ms')


def tick_query(server):
    """Asks `tick query` and reads the average and percentiles it prints (both servers print the same)."""
    start = server.count()
    server.console('tick query')
    deadline = time.time() + 5
    average = percentiles = None
    while time.time() < deadline and (average is None or percentiles is None):
        for line in server.since(start):
            if (m := TICK_AVERAGE.search(line)) and average is None:
                average = float(m.group(1))
            if (m := TICK_PERCENTILES.search(line)) and percentiles is None:
                percentiles = tuple(float(g) for g in m.groups())
        time.sleep(0.05)
    if average is None or percentiles is None:
        return None
    return {'average': average, 'p50': percentiles[0], 'p95': percentiles[1], 'p99': percentiles[2]}


def server_command(kind, directory, args):
    if kind == 'vanilla':
        _, jars, libraries = base.verify_artifacts()
        classpath = os.pathsep.join(map(str, [jars['server'], *libraries]))
        return ['java', f'-Xms{args.heap}', f'-Xmx{args.heap}', '-Duser.timezone=UTC', '-cp', classpath,
                'net.minecraft.server.Main', 'nogui']
    server = BIN / f'horizon-server{EXE}'
    subprocess.run([server, '--dir', directory, 'import', directory / 'server.properties'], check=True,
                   capture_output=True)
    toml = directory / 'server.toml'
    # The same settings, without the web panel.
    toml.write_text(re.sub(r'(?m)^bind = "127\.0\.0\.1:8130"', 'bind = ""', toml.read_text()))
    return [server, '--dir', directory, 'run']


def run_once(kind, bots, args, out):
    directory = out / f'{kind}-{bots}'
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir(parents=True)
    port = free_port()
    props = dict(PROPERTIES, **{'server-port': str(port), 'level-seed': str(args.seed)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    command = server_command(kind, directory, args)
    started = time.time()
    server = Server(command, directory, out / f'{kind}-{bots}.log')
    result = {'server': kind, 'bots': bots}
    try:
        server.wait_for(r'/INFO\]: Done \(', 600)
        result['start_seconds'] = round(time.time() - started, 2)
        time.sleep(5)
        report = out / f'{kind}-{bots}-bots.json'
        total = bots * args.join_every + args.warmup + args.seconds
        bot_process = subprocess.Popen(
            [BIN / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', str(bots),
             '--behaviour', args.behaviour, '--join-every', str(args.join_every), '--seconds', str(int(total)),
             '--seed', str(args.seed), '--report', report],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        bot_lines = []
        threading.Thread(target=lambda: bot_lines.extend(bot_process.stdout), daemon=True).start()
        # All bots in, then the warm-up.
        time.sleep(bots * args.join_every + args.warmup)
        measure_start = time.time()
        rss, cpu, ticks = [], [], []
        _, last_cpu = tree_usage(server.process.pid)
        last_time = time.time()
        next_query = time.time()
        while time.time() - measure_start < args.seconds:
            time.sleep(1)
            now = time.time()
            r, c = tree_usage(server.process.pid)
            rss.append(r)
            cpu.append((c - last_cpu) / (now - last_time) * 100)
            last_cpu, last_time = c, now
            if now >= next_query:
                if (t := tick_query(server)) is not None:
                    ticks.append(t)
                next_query = now + 5
        measured = [json.loads(l) for l in list(bot_lines) if l.startswith('{')]
        window = [l for l in measured if l['t'] >= measured[-1]['t'] - args.seconds] if measured else []
        bot_process.wait(timeout=total + 120)
        summary = json.loads(report.read_text()) if report.exists() else {}
        mean = lambda xs: round(statistics.fmean(xs), 2) if xs else None
        result.update({
            'mspt': {k: mean([t[k] for t in ticks]) for k in ('average', 'p50', 'p95', 'p99')},
            'mspt_worst_p99': max((t['p99'] for t in ticks), default=None),
            # /tick query reports times, not the rate: 20 while the average fits in 50 ms.
            'tps': round(min(20.0, 1000 / max(statistics.fmean([t['average'] for t in ticks]), 1e-9)), 2) if ticks else None,
            'rss_mb': {'mean': mean([r / 1e6 for r in rss]), 'max': round(max(rss) / 1e6, 1) if rss else None},
            'cpu_percent': mean(cpu),
            'chunks_per_second': mean([l['chunks_per_second'] for l in window]),
            'bots_joined': summary.get('joined'),
            'bots_failed': [b for b in summary.get('per_bot', []) if b.get('error')],
            'tick_queries': len(ticks),
        })
    except Exception as e:  # a failed run is reported, and the next one still runs
        result['error'] = str(e)
    finally:
        server.stop()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--servers', default='horizon,vanilla')
    parser.add_argument('--bots', default='1,10,25,50')
    parser.add_argument('--behaviour', default='mixed')
    parser.add_argument('--join-every', type=float, default=1.0)
    parser.add_argument('--warmup', type=int, default=60)
    parser.add_argument('--seconds', type=int, default=300)
    parser.add_argument('--seed', type=int, default=12345)
    parser.add_argument('--heap', default='4g', help='the Java heap, -Xms and -Xmx (vanilla)')
    parser.add_argument('--out', type=Path, default=None, help='the report (JSON)')
    args = parser.parse_args()
    out = lab.LOCAL / 'server-bench'
    out.mkdir(parents=True, exist_ok=True)
    runs = []
    for bots in [int(b) for b in args.bots.split(',')]:
        for kind in args.servers.split(','):
            print(f'{kind}, {bots} bots...', flush=True)
            result = run_once(kind, bots, args, out)
            runs.append(result)
            print(json.dumps(result), flush=True)
    report = {
        'kind': 'horizon-server-benchmark',
        'date': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'machine': {'system': platform.system(), 'machine': platform.machine(), 'cpus': os.cpu_count()},
        'commit': subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, capture_output=True, text=True).stdout.strip(),
        'java': subprocess.run(['java', '--version'], capture_output=True, text=True).stdout.splitlines()[:1],
        'settings': {**vars(args), 'out': str(args.out), 'properties': PROPERTIES},
        'runs': runs,
    }
    path = args.out or out / 'report.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(f'\n{"server":8} {"bots":>4} {"start s":>8} {"MSPT":>6} {"P95":>6} {"P99":>6} {"RSS MB":>7} {"CPU %":>6} {"chunks/s":>8} {"joined":>6}')
    for r in runs:
        if 'error' in r:
            print(f'{r["server"]:8} {r["bots"]:>4}  failed: {r["error"].splitlines()[0]}')
            continue
        print(f'{r["server"]:8} {r["bots"]:>4} {r["start_seconds"]:>8} {r["mspt"]["average"]!s:>6} {r["mspt"]["p95"]!s:>6} '
              f'{r["mspt"]["p99"]!s:>6} {r["rss_mb"]["mean"]!s:>7} {r["cpu_percent"]!s:>6} {r["chunks_per_second"]!s:>8} '
              f'{r["bots_joined"]!s:>6}')
    print(f'\nWrote {path}')


if __name__ == '__main__':
    sys.exit(main())
