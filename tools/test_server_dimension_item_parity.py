#!/usr/bin/env python3
"""An item teleported into another level from a dedicated server's console, against vanilla's.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat
world vanilla generated first. With no player online, the console force-loads the chunk at 0, 0 of the Overworld
and of the Nether, clears the air there, and summons a floating stick item (a fixed UUID, no gravity, moving) in
the Overworld. It then runs `execute in minecraft:the_nether run tp @e[type=minecraft:item] 0.5 100 0.5`:
`TeleportCommand.performTeleport` -> `Entity.teleportTo(ServerLevel, ..)`, whose transition has no motion and no
`Relative.DELTA`. Before and after, each level's items are listed from the console (how many, then each one's
UUID, item, position and motion). Every reply, the teleport's own included, must be the same on both servers.
"""
import os
import platform
import re
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, wait_ticks
    from .test_forced_end_parity import answer
    from .test_locator_bar_parity import generate_world, server_dir
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, wait_ticks
    from test_forced_end_parity import answer
    from test_locator_bar_parity import generate_world, server_dir

NAME = 'server-dimension-item-parity'
LEVELS = ('minecraft:overworld', 'minecraft:the_nether')
ITEM = '@e[type=minecraft:item,distance=0..]'
SUMMON = ('summon minecraft:item 0.5 100 0.5 {UUID:[I;1,2,3,4],NoGravity:1b,PickupDelay:32767s,Age:-32768s,'
          'Motion:[0.2d,0.0d,0.0d],Item:{id:"minecraft:stick",count:3}}')
TELEPORT = 'execute in minecraft:the_nether run tp @e[type=minecraft:item] 0.5 100 0.5'


def items(fields):
    """Each level's items: how many (`distance=0..` keeps the selector to the level `execute in` names), then
    `fields` of each one."""
    queries = []
    for level in LEVELS:
        queries.append(f'execute in {level} if entity {ITEM}')
        queries += [f'execute in {level} as {ITEM} run data get entity @s {field}' for field in fields]
    return queries


# (step, console commands whose replies are compared). The item drifts until the teleport, so its position and
# motion are compared only after it.
SCRIPT = [
    ('before', items(('UUID', 'Item'))),
    ('teleport', [TELEPORT]),
    ('after', items(('UUID', 'Item', 'Pos', 'Motion'))),
]


def run_side(label, command, build, world):
    directory = build / label
    server_dir(directory, world)
    server = Server(command, directory, label)
    replies = {}
    try:
        server.wait_for(r'Done \(')
        for level in LEVELS:
            server.console(f'execute in {level} run forceload add 0 0')
        for poll in range(300):
            if all('passed' in answer(server, f'execute in {level} if loaded 0 100 0', f'loaded {poll} {level}')
                   for level in LEVELS):
                break
            wait_ticks(server, 20)
        else:
            raise AssertionError(f'{label}: the chunks at 0, 0 never loaded')
        for level in LEVELS:
            answer(server, f'execute in {level} run fill -2 98 -2 2 102 2 minecraft:air', f'cleared {level}')
        answer(server, SUMMON, 'summoned')
        n = 0
        for step, commands in SCRIPT:
            for command_line in commands:
                replies[(step, command_line)] = answer(server, command_line, f'{step} {n}')
                n += 1
            wait_ticks(server, 20)
    finally:
        server.stop()
    return replies


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-game', '--bin', 'horizon-client'],
                   cwd=lab.ROOT, check=True, capture_output=True)
    binary = lab.cargo_target() / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    world = generate_world(vanilla_server, build)
    vanilla = run_side('vanilla', vanilla_server, build, world)
    horizon = run_side('horizon', horizon_server, build, world)
    normal = lambda s: re.sub(r'\s+', ' ', s).strip()
    problems = [f'{step}: {command}\n  vanilla: {vanilla[step, command]}\n  horizon: {horizon[step, command]}'
                for step, command in vanilla if normal(vanilla[step, command]) != normal(horizon[step, command])]
    if problems:
        raise AssertionError('the item teleport differs:\n' + '\n'.join(problems))
    if 'passed' not in vanilla[('after', f'execute in {LEVELS[1]} if entity {ITEM}')]:
        raise AssertionError(f'vanilla left no item in the Nether: the script did not run (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        replies={f'{step}: {command}': reply for (step, command), reply in vanilla.items()},
        scope='An item teleported from the Overworld into the Nether by the console (`execute in ... run tp`): '
              'both levels\' items before and after, and the teleport\'s reply.'))
    print(f'server dimension item: {len(vanilla)} replies identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
