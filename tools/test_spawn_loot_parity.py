#!/usr/bin/env python3
"""A saved spawn chest unpacks its loot on the first inventory access.

Containers keep their loot table and seed until the first inventory access.
Loading the world preserves an unopened chest without generating its items.

The vanilla 26.3 server writes a flat world with a seeded loot chest beside the world spawn, then
rolls the chest itself in a copy of that world (`execute if items` unpacks it). Horizon opens the
untouched world in a new process, performs the same inventory query, and must then hold the same
items in the same slots.

  test_spawn_loot_parity.py [--bin-dir DIR] [--keep]
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_save_roundtrip_parity import Vanilla  # noqa: E402
from test_world_reopen_parity import game_env  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
RESOURCES = ROOT / '.local' / 'horizon-client' / '26.3'
RUN = ROOT / '.local' / 'spawn-loot'
CHEST = (2, -60, 1)
LOOT = '{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}'
MODES = {'server thread': []}


def vanilla_world():
    """The world with the unopened chest, and what vanilla rolls from it: [(slot, id, count)]."""
    server = Vanilla(RUN / 'v0')
    try:
        server.run('setworldspawn 0 -60 0')
        server.run('forceload add 0 0')
        time.sleep(2)
        server.run('setblock {} {} {} minecraft:chest{}'.format(*CHEST, LOOT))
        server.run('save-all flush')
        time.sleep(2)
    finally:
        server.stop()
    shutil.copytree(RUN / 'v0', RUN / 'vv')
    server = Vanilla(RUN / 'vv')
    try:
        server.run('forceload add 0 0')
        time.sleep(2)
        server.run('execute if items block {} {} {} container.* *'.format(*CHEST))
        text = ' '.join(server.run('data get block {} {} {} Items'.format(*CHEST)))
    finally:
        server.stop()
    rolled = []
    for item in re.finditer(r'\{(?:[^{}]|\{(?:[^{}]|\{[^{}]*\})*\})*\}', text.split('data:', 1)[-1]):
        found = item.group(0)
        count = re.search(r'count: (\d+)', found)
        rolled.append((int(re.search(r'Slot: (\d+)b', found).group(1)), re.search(r'id: "([^"]+)"', found).group(1),
                       int(count.group(1)) if count else 1))
    return sorted(rolled)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build them with the fast profile)')
    parser.add_argument('--keep', action='store_true', help='keep the run folder')
    args = parser.parse_args()
    bin_dir = Path(args.bin_dir).resolve() if args.bin_dir else None
    if bin_dir is None:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-game', '-p', 'horizon-client-lib', '--bins'], cwd=ROOT, check=True)
        bin_dir = ROOT / os.environ.get('CARGO_TARGET_DIR', 'target') / 'fast'
    shutil.rmtree(RUN, ignore_errors=True)
    RUN.mkdir(parents=True)
    expected = vanilla_world()
    print('vanilla rolls:', expected)
    if not expected:
        sys.exit('vanilla rolled nothing from the chest')
    env, extra = game_env()
    failures = 0
    for index, (mode, flags) in enumerate(MODES.items()):
        folder = RUN / f'h{index}'
        shutil.copytree(RUN / 'v0' / 'world', folder / 'saves' / 'world')
        socket = Path(tempfile.mkdtemp(prefix='hzsl')) / 'control.sock'  # sockets need short paths

        def ctl(*words, check=True):
            result = subprocess.run([str(bin_dir / 'horizon-ctl'), '--socket', str(socket), '--timeout', '600',
                                     *map(str, words)], cwd=folder, env=env, capture_output=True, text=True)
            if check and result.returncode:
                sys.exit(f'horizon-ctl {" ".join(map(str, words))} failed: {result.stdout}{result.stderr}')
            return json.loads(result.stdout) if result.stdout.strip().startswith('{') else result.stdout

        ctl('launch', '--resources', RESOURCES, '--world', 'world', '--render-distance', '4', *flags, *extra)
        try:
            ctl('wait-ready', '300')
            ctl('command', 'execute if items block {} {} {} container.* *'.format(*CHEST))
            slots = ctl('container', *CHEST)['slots']
        finally:
            ctl('quit', check=False)
        got = sorted((slot, stack['item'], stack['count']) for slot, stack in enumerate(slots) if stack)
        ok = got == expected
        failures += not ok
        print(f'{mode}: {len(got)} stacks', 'ok' if ok else f'FAIL: {got}')
    if not args.keep and not failures:
        shutil.rmtree(RUN, ignore_errors=True)
    print('spawn loot:', 'PASS' if not failures else f'{failures} FAILED')
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
