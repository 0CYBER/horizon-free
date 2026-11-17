#!/usr/bin/env python3
"""Fixed scene on a `state-digest` dedicated server: BINARY RESOURCES TEMPLATE OUT [--detail] [--ticks N] [--sprint] [--fast-step] [--random-ticks] [--no-digest] [--generate]; random ticks are off unless --random-ticks."""
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import lab

GATE = os.environ.get('HORIZON_SCENE_GATE')  # optional script that exits 0 when the machine is free to run
PROPERTIES = {
    'online-mode': 'false', 'level-seed': '12345', 'level-type': 'minecraft\\:flat', 'generate-structures': 'false',
    'gamemode': 'survival', 'difficulty': 'normal', 'view-distance': '4', 'simulation-distance': '4',
    'spawn-protection': '0', 'pause-when-empty-seconds': '-1', 'spawn-monsters': 'false', 'spawn-animals': 'false',
    'spawn-npcs': 'false', 'max-tick-time': '-1',
}
SPECIES = ['pig', 'cow', 'sheep', 'chicken', 'rabbit', 'zombie', 'skeleton', 'spider', 'villager', 'wolf']
Y = -60  # the flat world's surface
AREA = (0, 0, 95, 95)  # forceloaded block area: 6 x 6 chunks
SYNC = (424242, 100000, 987654321)  # uniquifier, entity id, level seed


def scene_commands(random_ticks):
    commands = [f'digest-sync {SYNC[0]} {SYNC[1]} {SYNC[2]}']
    if not random_ticks:
        commands.append('gamerule random_tick_speed 0')
    for s, species in enumerate(SPECIES):
        for i in range(20):
            x, z = 8 + s * 8 + (i % 4) * 1.5, 8 + (i // 4) * 1.5
            commands.append(f'summon minecraft:{species} {x:.1f} {Y} {z:.1f}')
    for i in range(20):
        x, z = 8 + i * 4, 60
        commands.append(f'setblock {x} {Y} {z} minecraft:chest')
        commands.append(f'setblock {x} {Y + 1} {z} minecraft:hopper[facing=down]'
                        '{Items:[{Slot:0b,id:"minecraft:cobblestone",count:64},{Slot:1b,id:"minecraft:dirt",count:32}]}')
    for i in range(5):
        commands.append(f'summon minecraft:tnt {10 + i * 15}.5 {Y} 85.5 {{fuse:{300 + i * 10}s}}')
    for i in range(10):
        commands.append(f'summon minecraft:item {8 + i * 8}.5 {Y + 3} 70.5 '
                        f'{{Item:{{id:"minecraft:{"diamond" if i % 2 else "oak_log"}",count:{i + 1}}}}}')
    return commands


class Server:
    def __init__(self, binary, resources, directory, env, freeze=False):
        self.lines = []
        self.log = open(directory / 'server.log', 'w')
        self.process = subprocess.Popen(
            [binary, '--resources', resources, '--dedicated', '--nogui'], cwd=directory, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        threading.Thread(target=self._read, daemon=True).start()
        self.count = 0
        if freeze:
            # Queued before the first tick, so no unfrozen tick moves the game time by wall clock.
            self.process.stdin.write('tick freeze\n')
            self.process.stdin.flush()

    def _read(self):
        for line in self.process.stdout:
            self.lines.append(line.rstrip('\n'))
            self.log.write(line)
            self.log.flush()

    def wait_for(self, text, start=0, timeout=300):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if any(text in line for line in self.lines[start:]):
                return
            if self.process.poll() is not None:
                raise SystemExit('server exited:\n' + '\n'.join(self.lines[-30:]))
            time.sleep(0.01)
        raise SystemExit(f'no "{text}" in {timeout} s')

    def run(self, commands, timeout=300):
        self.count += 1
        mark = f'DIGESTMARK{self.count}X'
        start = len(self.lines)
        self.process.stdin.write(''.join(c + '\n' for c in commands) + f'say {mark}\n')
        self.process.stdin.flush()
        self.wait_for(mark, start, timeout)
        return self.lines[start:]

    def game_time(self):
        for line in reversed(self.run(['time query gametime'])):
            if 'time is ' in line:
                return int(line.split('time is ')[1].split()[0])
        raise SystemExit('no game time')

    def stop(self):
        if self.process.poll() is None:
            self.process.stdin.write('stop\n')
            self.process.stdin.flush()
            try:
                self.process.wait(120)
            except subprocess.TimeoutExpired:
                self.process.kill()


def hold_gate():
    if not GATE:
        return
    env = dict(os.environ, GATE_OWNER='state-digest')
    while subprocess.run([sys.executable, GATE], env=env, capture_output=True).returncode != 0:
        time.sleep(10)


def scratch_env(home, digest, detail):
    folders = {}
    for key in ('HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'):
        path = home / key.lower()
        path.mkdir(parents=True, exist_ok=True)
        folders[key] = path
    env = lab.game_env(**folders, HORIZON_TEST_SEEDS='1', DISPLAY=':99')
    if digest:
        env['HORIZON_STATE_DIGEST'] = str(digest)
    if detail:
        env['HORIZON_STATE_DIGEST_DETAIL'] = '1'
    return env


def prepare_dir(directory):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in PROPERTIES.items()))
    (directory / 'eula.txt').write_text('eula=true\n')


def main():
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    binary, resources, template, out = map(Path, args[:4])
    ticks = int(sys.argv[sys.argv.index('--ticks') + 1]) if '--ticks' in sys.argv else 200
    out.mkdir(parents=True, exist_ok=True)
    home = Path(tempfile.mkdtemp(prefix='state-digest-home-'))
    hold_gate()
    if '--generate' in sys.argv:
        prepare_dir(template)
        server = Server(binary, resources, template, scratch_env(home, None, False))
        try:
            server.wait_for('Done (')
            server.run([f'forceload add {AREA[0]} {AREA[1]} {AREA[2]} {AREA[3]}'])
            time.sleep(5)
        finally:
            server.stop()
        return
    run_dir = out / 'server'
    if run_dir.exists():
        shutil.rmtree(run_dir)
    prepare_dir(run_dir)
    shutil.copytree(template / 'world', run_dir / 'world')
    (run_dir / 'world' / 'session.lock').unlink(missing_ok=True)
    digest = out / 'digest.txt'
    server = Server(binary, resources, run_dir, scratch_env(home, None if '--no-digest' in sys.argv else digest, '--detail' in sys.argv), freeze=True)
    timings = {}
    try:
        server.wait_for('Done (')
        server.run(['forceload query'])
        time.sleep(3)
        server.run(scene_commands('--random-ticks' in sys.argv))
        before = server.game_time()
        start = time.time()
        if '--sprint' in sys.argv:
            at = len(server.lines)
            server.run(['tick unfreeze', f'tick sprint {ticks}'])
            server.wait_for('Sprint completed', at)
            timings['sprint'] = next(line for line in server.lines[at:] if 'Sprint completed' in line).split(': ', 1)[-1]
        else:
            server.run((['tick rate 10000'] if '--fast-step' in sys.argv else []) + [f'tick step {ticks}'])
            while server.game_time() < before + ticks:
                time.sleep(0.05)
        timings['step_seconds'] = time.time() - start
    finally:
        server.stop()
        shutil.rmtree(home, ignore_errors=True)
    (out / 'timings.txt').write_text(f'{timings}\n')
    print(out, timings)


if __name__ == '__main__':
    main()
