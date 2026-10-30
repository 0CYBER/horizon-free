#!/usr/bin/env python3
"""The dedicated server's anticheat (docs/anticheat.md), end to end: a real `horizon-client --dedicated`
and horizon-bots, legit bots beside cheating ones.

    python3 tools/anticheat_e2e.py --bin-dir target/fast            # every check
    python3 tools/anticheat_e2e.py --bin-dir target/fast --check timer

Checks:
- off:   no `[anticheat]` in server.toml: the anticheat says nothing, writes no log and kicks no one, the
         cheating bot included (vanilla's checks only).
- timer: `[anticheat]` on: a bot ticking 1.5 times as fast as vanilla's client is flagged, sent back,
         reported to the console and kicked; the legit bots (walking and exploring at 20 Hz) are never
         flagged.

Each server runs in a fresh folder with fresh HORIZON_HOME and XDG folders (lab.game_env), on a free port,
in a flat creative world (the bots fly). Build first:
    cargo build --profile fast -p horizon-client --bin horizon-client
    cargo build --profile fast -p horizon-bots
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'spawn-protection': '0',
    'view-distance': '4',
    'simulation-distance': '4',
    'max-players': '100',
}
ON = '''[anticheat]
enabled = true
'''


class Failed(Exception):
    pass


def start(bins, folder, anticheat, out, name):
    folder.mkdir(parents=True)
    port = lab.free_port()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (folder / 'eula.txt').write_text('eula=true\n')
    if anticheat is not None:
        (folder / 'server.toml').write_text(anticheat)
    command = [bins / f'horizon-client{EXE}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    server = Server(command, folder, out / f'{name}-server.log')
    server.wait_for(DONE, 300)
    return server, port


def bots(bins, port, out, name, count, behaviour, seconds, *extra):
    """horizon-bots, run to the end; its report."""
    report = out / f'{name}.json'
    subprocess.run([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', str(count),
                    '--behaviour', behaviour, '--seconds', str(seconds), '--join-every', '0.2',
                    '--name', name, '--report', report, *extra],
                   check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=seconds + 180)
    return json.loads(report.read_text())


def run_pair(bins, folder, out, name, anticheat, seconds):
    """Legit bots and a timer cheat on one server, at the same time; (server lines, legit, cheat, log)."""
    server, port = start(bins, folder, anticheat, out, name)
    try:
        with ThreadPoolExecutor(2) as pool:
            legit = pool.submit(bots, bins, port, out, f'{name}Legit', 6, 'mixed', seconds)
            cheat = pool.submit(bots, bins, port, out, f'{name}Timer', 1, 'walk', seconds, '--timer', '1.5')
            legit, cheat = legit.result(), cheat.result()
    finally:
        server.stop()
    log = folder / 'logs/anticheat.log'
    return server.since(0), legit, cheat, log.read_text() if log.exists() else None


def check_off(bins, scratch, out, seconds):
    lines, legit, cheat, log = run_pair(bins, scratch / 'off', out, 'Off', None, seconds)
    if log is not None:
        raise Failed(f'off: an anticheat log was written:\n{log}')
    noise = [l for l in lines if 'Anticheat' in l]
    if noise:
        raise Failed('off: the anticheat spoke:\n' + '\n'.join(noise))
    for bot in legit['per_bot'] + cheat['per_bot']:
        if bot['error']:
            raise Failed(f'off: {bot["name"]} was dropped: {bot["error"]}')
    print(f'off: {legit["joined"] + cheat["joined"]} bots, the 1.5x one included, never flagged or kicked')


def check_timer(bins, scratch, out, seconds):
    lines, legit, cheat, log = run_pair(bins, scratch / 'timer', out, 'On', ON, seconds)
    if not any('Anticheat on' in l for l in lines):
        raise Failed('timer: the server did not say the anticheat is on')
    log = log or ''
    for bot in legit['per_bot']:
        if bot['name'] in log:
            raise Failed(f'timer: legit {bot["name"]} was flagged:\n{log}')
        if bot['error']:
            raise Failed(f'timer: legit {bot["name"]} was dropped: {bot["error"]}')
    cheater = cheat['per_bot'][0]
    flags = log.count(f'{cheater["name"]} failed timer')
    if flags < 3:
        raise Failed(f'timer: the 1.5x bot was flagged {flags} times:\n{log}')
    if 'sent back' not in log or cheater['pulled_back'] == 0:
        raise Failed('timer: the 1.5x bot was never sent back')
    if not any(f'[Anticheat] {cheater["name"]} failed timer' in l for l in lines):
        raise Failed('timer: no alert on the console')
    if 'kicked' not in log or 'disconnected' not in (cheater['error'] or ''):
        raise Failed(f'timer: the 1.5x bot was not kicked ({cheater["error"]})')
    print(f'timer: 1.5x bot flagged {flags} times, sent back {cheater["pulled_back"]} times, alerted and '
          f'kicked; {legit["joined"]} legit bots never flagged')


CHECKS = {'off': check_off, 'timer': check_timer}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--check', action='append', choices=sorted(CHECKS))
    parser.add_argument('--seconds', type=int, default=40)
    parser.add_argument('--out', default=str(lab.LOCAL / 'anticheat-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    # One set of scratch HORIZON_HOME and XDG folders for every server below.
    os.environ.update(lab.game_env())
    failed = []
    with tempfile.TemporaryDirectory(prefix='anticheat-e2e-') as scratch:
        for name in args.check or sorted(CHECKS):
            try:
                CHECKS[name](bins, Path(scratch), out, args.seconds)
            except (Failed, RuntimeError) as e:
                failed.append(name)
                print(f'FAILED {e}')
    if failed:
        sys.exit(f'anticheat_e2e: {", ".join(failed)} failed (logs in {out})')


if __name__ == '__main__':
    main()
