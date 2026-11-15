#!/usr/bin/env python3
"""Levels a data pack or a native mod adds, end to end in the real game (arismoko/horizon#1042).

The game runs headless and horizon-ctl drives it, on single player's server thread: the window is a
client of that server, so a block is read from both sides.

- pack: a world's data pack adds a level of a custom build height. `execute in ... run tp` takes the
  player there, onto terrain; a block below y=0 reaches the client and the server; the level saves and
  the player comes back to it. A second level names a generator nobody registered: the trips into it
  fail, for a player and for an entity, and the game plays on.
- mod: crates/horizon-mod-dimension-game, built as the launcher's generated `main` is, with one native mod,
  `dimprobe`, and its `horizon-native-mods/dimprobe` package beside the executable, and no data pack.
  `host.commands().run_command("execute in ... run tp ...")` takes the player into a `register_dimension`
  level from inside a mod's command, then into a `register_chunk_generator` level from a server tick.
  Both save and come back.
- replace: a data pack puts the mod's level on other terrain.
- dedicated: a headless client on a dedicated server is sent to a pack's level from the console, and to
  the level with no generator. Then that player's own level loses its generator: they join in the
  Overworld and the server stays up.

    python3 tools/mod_dimensions_e2e.py                   # all of them, about two minutes
    python3 tools/mod_dimensions_e2e.py --scenario mod
    python3 tools/mod_dimensions_e2e.py --bin-dir target/fast

Needs the prepared resources in .local/horizon-client. Each game gets fresh HORIZON_HOME, launcher and XDG
folders, a scratch game directory and no audio; only the logs in --out outlive the run.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from integration_env import run_game  # noqa: E402
from test_dedicated_server_parity import Server  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SEED = 42
# horizon-mod-dimension-game: the gold block over each chunk's corner, from the seed.
MARKER_Y = 1 + SEED % 16
PACK_META = {'pack': {'description': 'levels', 'min_format': [121, 0], 'max_format': [121, 0]}}
NOISE = {'type': 'minecraft:noise', 'settings': 'minecraft:overworld',
         'biome_source': {'type': 'minecraft:fixed', 'biome': 'minecraft:plains'}}
GOLD = 'minecraft:gold_block'
STONE = 'minecraft:stone'
PLAYER = 'Tester'
# The package the game checks its built mod against; its code is in the executable.
MANIFEST = f"""id = 'dimprobe'
name = 'dimprobe'
version = '1.0.0'
api = '0.2'
side = 'both'
capabilities = ['registries']
[native]
game = '26.3'
horizon = '{'a' * 40}'
mixins = '{'b' * 40}'
"""


class Failed(Exception):
    pass


class Session:
    """One scratch game folder and the games run in it, one at a time."""

    def __init__(self, bins, out, name):
        self.root = Path(tempfile.mkdtemp(prefix='mod-dims-'))
        self.game_dir = self.root / 'game'
        self.game_dir.mkdir()
        self.bins, self.out, self.name = bins, out, name
        self.binary = bins / 'horizon-client'
        self.socket = self.root / 'ctl.sock'
        self.env = lab.game_env({k: v for k, v in os.environ.items()
                                 if k not in ('HORIZON_HOME', 'HORIZON_LAUNCHER_HOME',
                                              'XDG_DATA_HOME', 'XDG_CONFIG_HOME')})
        self.failures = []

    def run(self, *words, timeout=30):
        return subprocess.run([str(self.bins / 'horizon-ctl'), '--socket', str(self.socket), *map(str, words)],
                              cwd=self.game_dir, env=self.env, capture_output=True, text=True,
                              timeout=timeout)

    def ctl(self, *words, timeout=30):
        """A horizon-ctl command's JSON answer; a refused one raises."""
        done = self.run(*words, timeout=timeout)
        if done.returncode:
            raise Failed(f'horizon-ctl {" ".join(map(str, words))}: {done.stdout}{done.stderr}')
        return json.loads(done.stdout)

    def launch(self):
        done = self.run('launch', '--world', 'W', '--creative', '--seed', SEED, '--headless', '--wait',
                        '--game', self.binary, '--resources', lab.RESOURCES, '--game-dir', self.game_dir,
                        timeout=180)
        if done.returncode:
            raise Failed(f'the game did not start: {done.stdout}{done.stderr}{self.log()[-2000:]}')

    def use_mod_game(self):
        """Run the game with the mod, from a scratch folder that holds its package."""
        folder = self.root / 'bin'
        package = folder / 'horizon-native-mods/dimprobe'
        (package / 'src').mkdir(parents=True)
        (package / 'horizon-mod.toml').write_text(MANIFEST)
        (package / 'Cargo.toml').write_text("[package]\nname = 'dimprobe'\nversion = '1.0.0'\n")
        (package / 'src/lib.rs').write_text('// Built into the executable.\n')
        self.binary = folder / 'horizon-client'
        shutil.copy2(self.bins / 'horizon-mod-dimension-game', self.binary)

    def quit(self):
        """Quit, saving, and wait until the game has gone."""
        self.run('quit')
        self.wait(lambda: self.run('state').returncode != 0, 60)

    def world(self):
        found = sorted(self.game_dir.rglob('level.dat'))
        if not found:
            raise Failed('no world folder')
        return found[0].parent

    def state(self):
        return self.ctl('state')

    def player(self):
        """The player's part of the state, empty between two levels."""
        return self.state().get('player') or {}

    def command(self, line):
        return self.ctl('command', line)

    def chat(self, line):
        """What a command answered, in one string: `ok` only says it reached the server."""
        return ' '.join(self.command(line).get('chat', []))

    def server_block(self, x, y, z, block):
        """Whether the server holds `block` there, by `execute if block`."""
        return 'Test passed' in self.chat(f'execute if block {x} {y} {z} {block}')

    def client_block(self, x, y, z):
        """The block the client holds there, None in a chunk it has not got."""
        answer = self.ctl('block', x, y, z)
        return answer['block'] if answer.get('loaded') else None

    def wait(self, test, seconds=45):
        """Poll `test` until it is true; False when the time runs out."""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if test():
                return True
            time.sleep(0.25)
        return False

    def arrived(self, dimension, x, z):
        """Wait until the client is in `dimension` over the block column x, z and no screen is up."""
        def there():
            state = self.state()
            at = self.player().get('block_pos', [None] * 3)
            return state['dimension'] == dimension and (at[0], at[2]) == (x, z) and state['screen'] == 'playing'
        return self.wait(there)

    def where(self):
        state = self.state()
        return f"{state['dimension']} at {self.player().get('position')} on {state['screen']}"

    def check(self, what, ok, detail=''):
        print(('PASS ' if ok else 'FAIL ') + what + ('' if ok else f': {detail}'), flush=True)
        if not ok:
            self.failures.append(what)

    def log(self):
        path = Path(self.env['HORIZON_HOME']) / 'logs/latest.log'
        return path.read_text(errors='replace') if path.is_file() else ''

    def close(self):
        self.quit()
        logs = Path(self.env['HORIZON_HOME']) / 'logs'
        if logs.is_dir():
            shutil.copytree(logs, self.out / self.name, dirs_exist_ok=True)
        shutil.rmtree(self.root, ignore_errors=True)


def write_pack(world, name, files):
    """A data pack in the world: `files` maps paths under data/ to their JSON."""
    pack = world / 'datapacks' / name
    lab.write_json(pack / 'pack.mcmeta', PACK_META)
    for path, value in files.items():
        lab.write_json(pack / 'data' / path, value)


def overworld_type(**fields):
    """The prepared Overworld dimension type with `fields`: a pack's type must name them all."""
    path = Path(lab.RESOURCES) / 'data/minecraft/dimension_type/overworld.json'
    return dict(json.loads(path.read_text()), **fields)


def stand(s, y):
    """Stand the player on a block at `y` in the level they are in, at the column 8, 8."""
    s.chat(f'setblock 8 {y} 8 {GOLD}')
    s.chat(f'tp @s 8 {y + 1} 8')
    return s.wait(lambda: s.player().get('on_ground') and s.player()['block_pos'][1] == y + 1)


def block_on_both_sides(s, label, x, y, z, block):
    s.check(f'{label}: the client has {block}', s.wait(lambda: s.client_block(x, y, z) == block),
            s.client_block(x, y, z))
    s.check(f'{label}: the server has {block}', s.server_block(x, y, z, block))


def pack_scenario(s):
    level = 'test:pocket'
    s.launch()
    world = s.world()
    s.quit()
    write_pack(world, 'levels', {
        'test/dimension_type/pocket.json': overworld_type(min_y=-128, height=448, logical_height=448),
        'test/dimension/pocket.json': {'type': level, 'generator': NOISE},
        'test/dimension/ghost.json': {'type': level, 'generator': {'type': 'test:nobody'}},
    })
    s.launch()
    s.command(f'execute in {level} run tp @s 8 120 8')
    s.check('execute in reaches the level', s.arrived(level, 8, 8), s.where())
    s.check('the terrain reaches the client', s.wait(lambda: s.client_block(8, 0, 8) not in (None, 'minecraft:air')),
            s.client_block(8, 0, 8))
    s.check('the player stands in the level', stand(s, 100), s.where())
    s.check('the build height ends at y=-128', 'out of this world' in s.chat(f'setblock 8 -129 8 {STONE}'))
    s.chat(f'setblock 8 -100 8 {GOLD}')
    block_on_both_sides(s, 'below y=0', 8, -100, 8, GOLD)
    s.check('the server check tells blocks apart', not s.server_block(8, -100, 8, 'minecraft:dirt'))
    s.quit()
    s.check('the level saves its own regions', any((world / 'dimensions/test/pocket/region').glob('*.mca')))
    s.launch()
    s.check('the player comes back to the level', s.arrived(level, 8, 8), s.where())
    s.check('the player comes back to the spot', s.player()['block_pos'][1] == 101, s.where())
    block_on_both_sides(s, 'reloaded', 8, -100, 8, GOLD)

    # A level no generator answers for: the trip fails where it is tried, and only there.
    print('a player sent to the level with no generator:', s.chat('execute in test:ghost run tp @s 40 120 40'))
    s.check('the failed open is logged',
            s.wait(lambda: 'could not open test:ghost' in s.log(), 10))
    s.check('the player stays where they were', s.arrived(level, 8, 8), s.where())
    s.chat('summon minecraft:pig 9 101 8')
    answer = s.chat('execute in test:ghost run tp @e[type=minecraft:pig,limit=1] 40 120 40')
    print('an entity sent there:', answer)
    s.check('the entity is refused', 'Teleported' not in answer, answer)
    s.chat(f'setblock 9 100 8 {GOLD}')
    block_on_both_sides(s, 'the server plays on', 9, 100, 8, GOLD)


def mod_scenario(s):
    pocket, slabs = 'dimprobe:pocket', 'dimprobe:slabs'
    s.use_mod_game()
    s.launch()
    world = s.world()
    s.check('the world has no data pack', not any((world / 'datapacks').glob('*')))
    answer = s.chat('dimprobe enter pocket')
    s.check("the mod's run_command ran, inside its command", 'dimprobe ran=true' in answer, answer)
    s.check("run_command reaches register_dimension's level", s.arrived(pocket, 8, 8), s.where())
    s.check('the noise terrain reaches the client',
            s.wait(lambda: s.client_block(8, 0, 8) not in (None, 'minecraft:air')), s.client_block(8, 0, 8))
    s.check('the build height ends at y=-128', 'out of this world' in s.chat(f'setblock 8 -129 8 {STONE}'))
    s.chat(f'setblock 8 -100 8 {GOLD}')
    block_on_both_sides(s, 'below y=0', 8, -100, 8, GOLD)

    s.chat('dimprobe later slabs')
    s.check("run_command from a server tick reaches the generator's level", s.arrived(slabs, 8, 8), s.where())
    s.check('the player lands on the mod\'s floor', s.wait(lambda: s.player().get('block_pos', [0, 0])[1] == 1), s.where())
    block_on_both_sides(s, 'the floor', 8, 0, 8, STONE)
    block_on_both_sides(s, 'the lowest block', 8, -32, 8, STONE)
    block_on_both_sides(s, 'the seed marker', 0, MARKER_Y, 0, GOLD)
    block_on_both_sides(s, 'the next chunk', 16, MARKER_Y, 16, GOLD)
    block_on_both_sides(s, 'no surface or features', 8, 5, 8, 'minecraft:air')
    # The floor stops at the level's own lowest block, above the Overworld's.
    s.check('the generator stops at y=-32', s.client_block(8, -33, 8) == 'minecraft:air', s.client_block(8, -33, 8))
    s.chat(f'setblock 8 95 8 {GOLD}')
    block_on_both_sides(s, 'the highest block', 8, 95, 8, GOLD)
    s.quit()
    for level in ('pocket', 'slabs'):
        s.check(f'{level} saves its own regions', any((world / f'dimensions/dimprobe/{level}/region').glob('*.mca')))
    s.launch()
    s.check('the player comes back to the generator\'s level', s.arrived(slabs, 8, 8), s.where())
    block_on_both_sides(s, 'reloaded', 8, 95, 8, GOLD)
    s.chat('dimprobe enter pocket')
    s.check('the first level opens again', s.arrived(pocket, 8, 8), s.where())
    block_on_both_sides(s, 'reloaded below y=0', 8, -100, 8, GOLD)


def replace_scenario(s):
    level = 'dimprobe:pocket'
    s.use_mod_game()
    s.launch()
    world = s.world()
    s.quit()
    write_pack(world, 'swap', {'dimprobe/dimension/pocket.json': {'type': level, 'generator': {
        'type': 'dimprobe:slab'}}})
    s.launch()
    s.chat('dimprobe enter pocket')
    s.check('the level opens', s.arrived(level, 8, 8), s.where())
    block_on_both_sides(s, "the pack's generator", 0, MARKER_Y, 0, GOLD)
    block_on_both_sides(s, "the mod's dimension type", 8, -128, 8, STONE)


def dedicated_scenario(s):
    directory = s.root / 'server'
    directory.mkdir()
    port = lab.free_port()
    properties = {'online-mode': 'false', 'white-list': 'false', 'spawn-protection': '0', 'view-distance': '3',
                  'simulation-distance': '3', 'pause-when-empty-seconds': '-1', 'server-port': port,
                  'level-type': 'minecraft\\:flat', 'generate-structures': 'false'}
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in properties.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    command = [s.binary, '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    reports = []

    def client():
        """A headless client's stay: it joins, chats, and leaves after a few seconds."""
        home = Path(tempfile.mkdtemp(prefix='mod-dims-client-', dir=s.root))
        report = home / 'report.json'
        with (s.root / 'client.log').open('a') as log:
            run_game([str(s.binary), '--resources', str(lab.RESOURCES), '--render-distance', '2',
                      '--server', f'127.0.0.1:{port}', '--name', PLAYER], cwd=home,
                     env=dict(os.environ, HORIZON_NET_REPORT=str(report), HORIZON_NET_QUIET=''),
                     stdout=log, stderr=subprocess.STDOUT, timeout=120, check=False)
        reports.append(json.loads(report.read_text()) if report.is_file() else {'error': 'no report'})

    def during_a_stay(server, *lines):
        """Run console `lines` while the client is on, and what the console said of the player's level."""
        joined = sum(f'{PLAYER} joined the game' in line for line in server.lines)
        thread = threading.Thread(target=client)
        thread.start()
        s.wait(lambda: sum(f'{PLAYER} joined the game' in line for line in server.lines) > joined
               or server.process.poll() is not None)
        time.sleep(1)
        seen = None
        if server.process.poll() is None:
            for line in lines:
                server.console(line)
            start = len(server.lines)
            server.console(f'data get entity {PLAYER} Dimension')
            s.wait(lambda: any('has the following entity data' in line for line in server.lines[start:]), 5)
            seen = ' '.join(line for line in server.lines[start:] if 'entity data' in line)
        thread.join()
        return seen

    def start(name):
        server = Server(command, directory, name)
        server.wait_for(r'Done \(')
        return server

    server = start('first')
    server.console(f'op {PLAYER}')
    server.wait_for(f'Made {PLAYER} a server operator')
    server.stop()
    world = directory / 'world'
    stem = {'type': 'test:pocket', 'generator': {'type': 'test:nobody'}}
    write_pack(world, 'levels', {
        'test/dimension_type/pocket.json': overworld_type(min_y=-128, height=448, logical_height=448),
        'test/dimension/pocket.json': {'type': 'test:pocket', 'generator': NOISE},
        'test/dimension/ghost.json': stem,
    })
    server = start('second')
    try:
        seen = during_a_stay(server, 'execute in test:ghost run tp Tester 40 120 40')
        s.check('a trip to the level with no generator leaves the server up', server.process.poll() is None)
        s.check('the player stays in the Overworld', seen and 'minecraft:overworld' in seen, seen)
        s.check('the client stayed connected', reports[-1].get('error') is None, reports[-1])
        seen = during_a_stay(server, f'gamemode creative {PLAYER}', 'execute in test:pocket run tp Tester 8 120 8')
        s.check('the console sends a client to the level', seen and 'test:pocket' in seen, seen)
    finally:
        server.stop()
    write_pack(world, 'levels', {'test/dimension/pocket.json': stem})
    server = start('third')
    try:
        seen = during_a_stay(server)
        s.check("a player whose level lost its generator doesn't stop the server", server.process.poll() is None,
                server.lines[-5:])
        s.check('they join in the Overworld', seen and 'minecraft:overworld' in seen, seen)
        s.check('their client plays', reports[-1].get('error') is None and reports[-1].get('chunks', 0) > 0, reports[-1])
    finally:
        server.stop()
    for name in ('first', 'second', 'third'):
        shutil.copy2(s.root / f'{name}.log', s.out / f'dedicated-{name}.log')


SCENARIOS = {'pack': pack_scenario, 'mod': mod_scenario, 'replace': replace_scenario, 'dedicated': dedicated_scenario}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='a build with horizon-client, horizon-ctl and '
                                          'horizon-mod-dimension-game (default: build them, fast profile)')
    parser.add_argument('--scenario', choices=list(SCENARIOS), action='append')
    parser.add_argument('--out', default=str(ROOT / '.local/mod-dimensions-e2e'), help='logs go here')
    args = parser.parse_args()
    shutil.rmtree(args.out, ignore_errors=True)
    (Path(args.out)).mkdir(parents=True)
    if args.bin_dir:
        bins = Path(args.bin_dir).resolve()
    else:
        subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-game', '-p', 'horizon-client-lib',
                        '-p', 'horizon-mod-dimension-game', '--bin', 'horizon-client', '--bin', 'horizon-ctl',
                        '--bin', 'horizon-mod-dimension-game'], cwd=ROOT, check=True)
        bins = Path(lab.cargo_target()).resolve() / 'fast'
    failures = []
    for name in args.scenario or SCENARIOS:
        print(f'== {name}', flush=True)
        s = Session(bins, Path(args.out), name)
        try:
            SCENARIOS[name](s)
        except (Failed, AssertionError, subprocess.SubprocessError) as error:
            s.check(f'{name} ran to its end', False, error)
        finally:
            s.close()
        failures += s.failures
    print(f'{len(failures)} failed' if failures else 'all passed')
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
