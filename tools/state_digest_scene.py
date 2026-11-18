#!/usr/bin/env python3
"""Fixed scene on a `state-digest` dedicated server: BINARY RESOURCES TEMPLATE OUT [--detail] [--ticks N] [--sprint] [--fast-step] [--random-ticks] [--no-digest] [--generate] [--extended]; random ticks are off unless --random-ticks; --extended adds commands, projectiles, breeding, conversions and explosions (step mode only)."""
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


def scene_commands(random_ticks, extended=False):
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
    if extended:
        commands += extended_setup()
    return commands


def extended_setup():
    """Extra subjects for --extended, made before the window in the area z 24 to 57. Entities carry tags so the window's commands find them."""
    y = Y
    c = ['gamerule spawn_mobs false', 'time set midnight']  # no natural spawns, and the skeletons do not burn
    def put(kind, x, z, nbt='', dy=0):
        c.append(f'summon minecraft:{kind} {x}.5 {y + dy} {z}.5 {nbt}'.rstrip())
    # breeding: three pairs, set in love by the window's first pass
    for kind, z in (('pig', 28), ('cow', 34), ('sheep', 40)):
        for x in (8, 10):
            put(kind, x, z, '{Tags:["love"],PersistenceRequired:1b}')
    # primed TNT that explodes inside the window, side by side so one pushes the other
    put('tnt', 20, 24, '{fuse:50s}')
    put('tnt', 21, 24, '{fuse:50s}')
    # skeletons that shoot an iron golem; zombies that go for a villager
    put('iron_golem', 30, 44, '{PersistenceRequired:1b}')
    for x in (28, 30, 32):
        put('skeleton', x, 32, '{Tags:["sk"],PersistenceRequired:1b}')
    for x in (60, 61):
        put('zombie', x, 30, '{Tags:["zb"],PersistenceRequired:1b}')
    put('villager', 60, 36, '{PersistenceRequired:1b}')
    # mobs that take effects, attributes, NBT changes and projectiles
    for x in (40, 42, 44, 46):
        put('cow', x, 28, '{Tags:["fx"],PersistenceRequired:1b}')
    for x in (70, 71, 72):
        put('pig', x, 48, '{Tags:["tp"],PersistenceRequired:1b}')
    # conversions: lightning turns a pig and a villager, a countdown turns a zombie villager, a zombie and a piglin
    put('pig', 80, 28, '{Tags:["pg"],PersistenceRequired:1b}')
    put('villager', 80, 40, '{Tags:["vl"],PersistenceRequired:1b}')
    put('zombie_villager', 70, 34, '{ConversionTime:100,PersistenceRequired:1b}')
    # in a walled pool two blocks deep: the conversion stops once the zombie's eyes leave the water
    c.append(f'fill 73 {y} 33 75 {y + 1} 35 minecraft:stone hollow')
    c.append(f'setblock 74 {y} 34 minecraft:water')
    c.append(f'setblock 74 {y + 1} 34 minecraft:water')
    put('zombie', 74, 34, '{DrownedConversionTime:60,PersistenceRequired:1b}')
    put('piglin', 66, 24, '{TimeInOverworld:280,PersistenceRequired:1b}')
    put('hoglin', 86, 24, '{TimeInOverworld:280,PersistenceRequired:1b}')
    # creepers that blow up beside a cow
    put('creeper', 50, 48, '{ignited:1b,Fuse:20s}')
    put('cow', 52, 48, '{PersistenceRequired:1b}')
    put('creeper', 90, 48, '{ignited:1b,Fuse:30s,powered:1b}')
    # a slime that splits when killed, a minecart and a boat with a rider, two jockeys, zombies for /kill
    put('slime', 40, 50, '{Size:2,PersistenceRequired:1b}')
    put('minecart', 56, 52, '{Motion:[0.3d,0.0d,0.1d]}')
    put('oak_boat', 60, 52, '{Tags:["mt"]}')
    put('zombie', 60, 52, '{Tags:["rd"],PersistenceRequired:1b}')
    put('chicken', 34, 56, '{Passengers:[{id:"minecraft:zombie",IsBaby:1b,PersistenceRequired:1b}],PersistenceRequired:1b}')
    put('spider', 40, 56, '{Passengers:[{id:"minecraft:skeleton",PersistenceRequired:1b}],PersistenceRequired:1b}')
    for x in (50, 51):
        put('zombie', x, 56, '{Tags:["kill"],PersistenceRequired:1b}')
    put('armor_stand', 44, 56)
    # a dispenser with arrows and eggs, pulsed by the window
    c.append(f'setblock 24 {y} 52 minecraft:dispenser[facing=east]'
             '{Items:[{Slot:0b,id:"minecraft:arrow",count:16},{Slot:1b,id:"minecraft:egg",count:8}]}')
    return c


def extended_plan():
    """Commands run inside the window, by tick offset. The server is frozen between passes, so each runs between the same two ticks every time."""
    y = Y
    plan = {
        0: ['execute as @e[tag=love] run data merge entity @s {InLove:600}',
            'effect give @e[tag=fx] minecraft:speed 30 1', 'effect give @e[tag=fx] minecraft:poison 10 0',
            'execute as @e[tag=fx] run attribute @s minecraft:movement_speed base set 0.4',
            'item replace entity @e[tag=sk] weapon.offhand with minecraft:shield',
            'item replace entity @e[tag=zb] armor.head with minecraft:iron_helmet',
            f'fill 8 {y} 54 12 {y} 56 minecraft:stone',
            'ride @e[tag=rd,limit=1] mount @e[tag=mt,limit=1]'],
        5: [f'setblock 16 {y + 3} 48 minecraft:sand', f'setblock 18 {y + 3} 48 minecraft:gravel',
            'summon minecraft:armor_stand 46.5 -60 56.5 {Tags:["as"],Invisible:1b}'],
        10: ['summon minecraft:lightning_bolt 80.5 -60 28.5', f'setblock 24 {y} 53 minecraft:redstone_block'],
        12: ['summon minecraft:lightning_bolt 80.5 -60 40.5'],
        15: [f'setblock 14 {y} 50 minecraft:water'],
        20: ['execute as @e[tag=fx] run data modify entity @s Motion set value [0.0d,0.5d,0.0d]',
             'execute as @e[tag=fx] run data modify entity @s Health set value 8.0f',
             'tp @e[tag=tp] 76.5 -60 52.5 90 0',
             'execute as @e[tag=sk] at @s run tp @s ~1 ~ ~1'],
        25: ['execute as @e[tag=fx] at @s run summon minecraft:snowball ~ ~2 ~ {Motion:[0.3d,0.2d,0.1d]}',
             'execute as @e[tag=fx] at @s run summon minecraft:arrow ~ ~3 ~ {Motion:[0.0d,-0.5d,0.1d]}',
             'execute as @e[tag=fx] at @s run summon minecraft:experience_orb ~ ~1 ~ {Value:3s}']
            + [f'summon minecraft:egg {52 + i}.5 -57 {44 + i}.5 {{Motion:[0.1d,0.2d,0.05d]}}' for i in range(6)],
        30: ['summon minecraft:fireball 40.5 -56 40.5 {Motion:[0.0d,-0.3d,0.0d],ExplosionPower:2b}',
             f'setblock 24 {y} 53 minecraft:air'],
        40: [f'fill 8 {y} 54 12 {y} 56 minecraft:air destroy',
             'execute as @e[tag=tp] run damage @s 2 minecraft:generic',
             'effect clear @e[tag=fx] minecraft:speed'],
        50: [f'setblock 24 {y} 53 minecraft:redstone_block', 'execute as @e[tag=fx] run data merge entity @s {Fire:60s}'],
        60: ['kill @e[type=minecraft:slime]'],
        70: ['kill @e[tag=kill]', 'execute as @e[tag=love] run damage @s 1 minecraft:generic'],
        100: [f'summon minecraft:tnt 84.5 {y} 56.5 {{fuse:40s}}', 'summon minecraft:creeper 50.5 -60 40.5 {ignited:1b,Fuse:20s}'],
    }
    return plan


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
    if '--extended' in sys.argv and '--generate' not in sys.argv:
        if '--sprint' in sys.argv:
            raise SystemExit('--extended needs step mode: a sprint cannot stop between the passes')
        if ticks < 1:
            raise SystemExit('--extended needs --ticks of at least 1')
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
    for stale in ('commands.log', 'timings.txt'):
        (out / stale).unlink(missing_ok=True)
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
        extended = '--extended' in sys.argv
        setup = server.run(scene_commands('--random-ticks' in sys.argv, extended))
        if extended:
            # Written before anything else can fail, so the setup's console output is never lost.
            log = list(setup)
            transcript = open(out / 'commands.log', 'w')

            def note(lines):
                transcript.write(''.join(l + '\n' for l in lines))
                transcript.flush()
            note(setup)
        before = server.game_time()
        start = time.time()
        if extended:
            if '--fast-step' in sys.argv:
                server.run(['tick rate 10000'])
            done = 0
            for at, commands in sorted(extended_plan().items()):
                if at >= ticks:
                    break
                if at > done:
                    server.run([f'tick step {at - done}'])
                    while server.game_time() < before + at:
                        time.sleep(0.05)
                    done = at
                got = server.run(commands)
                log += got
                note(got)
            server.run([f'tick step {ticks - done}'])
            while server.game_time() < before + ticks:
                time.sleep(0.05)
            transcript.close()
            bad = [l for l in log if any(w in l for w in ('Unknown', 'Expected', 'Incorrect', 'Invalid', 'Unable', 'Could not', 'Only one entity', 'rror'))]
            timings['command_errors'] = len(bad)
            for line in bad:
                print('command problem:', line, file=sys.stderr)
        elif '--sprint' in sys.argv:
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
