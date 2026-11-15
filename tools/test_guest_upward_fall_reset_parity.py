#!/usr/bin/env python3
"""A connected player that moves up while it falls ends its fall.

`ServerGamePacketListenerImpl.handlePlayerPositionChange` calls `ServerPlayer.resetFallDistance` for a position
above the last one: the fall distance goes back to 0, and `fall_from_height` is awarded for the distance fallen
until then.

A real protocol client joins a vanilla 26.3 dedicated server and Horizon's as a survival player in the air. The
world is frozen and stepped by hand. The client sends one position a block lower, then one half a block higher,
each followed by one world step. After each, the console reads the player's fall distance and position, and which
of three `fall_from_height` advancements it has: any fall, a fall of at most 1 block, a fall of about 5. The two
servers must answer the same.

    python3 tools/test_guest_upward_fall_reset_parity.py
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

import lab
import test_locator_bar_parity as net
from test_dedicated_server_parity import game_time
from test_disconnect_crafting_parity import generate
from test_machine_recipe_placement_parity import Commands

PLAYER = 'FallGuest'
ADVANCEMENTS = {'any': None, 'short': {'max': 1.0}, 'five': {'min': 4.9, 'max': 5.1}}
MOVES = [('down', 109.0), ('up', 109.5)]


def pack(world):
    root = world / 'datapacks/fall'
    root.mkdir(parents=True)
    (root / 'pack.mcmeta').write_text(json.dumps({'pack': {
        'description': 'fall reset', 'pack_format': 121, 'min_format': [121, 0], 'max_format': [121, 0]}}))
    for name, distance in ADVANCEMENTS.items():
        criterion = {'trigger': 'minecraft:fall_from_height'}
        if distance:
            criterion['conditions'] = {'distance': {'y': distance}}
        path = root / 'data/fall/advancement' / f'{name}.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({'criteria': {name: criterion}}))


def step(server):
    """One world step; the player's own ticks go on while the world is frozen."""
    before = game_time(server)
    server.console('tick step 1')
    deadline = time.monotonic() + 30
    while game_time(server) < before + 1:
        assert time.monotonic() < deadline, 'the world step did not happen'
        time.sleep(.02)


def observe(commands):
    """What the console says of the player: its fall distance, its position and its advancements."""
    def data(path):
        text = commands.reply(f'data get entity {PLAYER} {path}')
        return re.search(r'has the following entity data: (.*)', text).group(1).strip()
    done = {name: 'Test passed' in commands.reply(
        f'execute if entity @a[name={PLAYER},advancements={{fall:{name}=true}}]') for name in ADVANCEMENTS}
    return {'fall_distance': data('fall_distance'), 'Pos': data('Pos'), 'advancements': done}


def run(side, command, client_command, out, world):
    directory = out / side
    port = net.server_dir(directory, world)
    server = net.Server(command, directory, side)
    commands = Commands(server, out / f'{side}-commands.json')
    client = None
    seen = {}
    with (out / f'{side}-client.log').open('w') as log:
        try:
            server.wait_for(r'Done \(')
            commands.reply('tick freeze')
            client = net.Client(client_command, port, out / f'{side}-wire.txt', log)
            for option in ['chat on', 'flat-chat on', 'state on', 'players on']:
                client.tell(option)
            client.tell('join ' + PLAYER)
            client.sync([PLAYER])
            for text in ['gamerule show_advancement_messages false', f'gamemode survival {PLAYER}',
                         f'tp {PLAYER} 12.5 110 12.5 0 0']:
                commands.reply(text)
            client.sync([PLAYER])
            step(server)
            seen['start'] = observe(commands)
            for label, y in MOVES:
                client.tell(f'fallmove {PLAYER} 12.5 {y} 12.5 false')
                client.sync([PLAYER])
                step(server)
                client.sync([PLAYER])
                seen[label] = observe(commands)
            client.end()
            client = None
        finally:
            if client:
                client.kill()
            server.stop()
    return seen


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Horizon game (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'guest-upward-fall-reset-parity')
    args = parser.parse_args()
    lock, jars, libraries = net.base.verify_artifacts()
    out = args.out.resolve()
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    classes = out / 'classes'
    classes.mkdir()
    source = lab.ROOT / 'mods/horizon/src/test/java'
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn',
                    '-sourcepath', source, '-cp', cp, '-d', classes,
                    source / 'dev/horizonlab/parity/LocatorBarClient.java'], check=True)
    client = ['java', '-Xmx512m', '-cp', str(classes) + os.pathsep + cp, 'dev.horizonlab.parity.LocatorBarClient']
    vanilla = ['java', '-Xmx1g', '-Duser.timezone=UTC', '-cp', cp, 'net.minecraft.server.Main', 'nogui']
    if args.binary:
        binary = args.binary.resolve(strict=True)
    else:
        subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'],
                       cwd=lab.ROOT, check=True)
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
        binary = Path(metadata['target_directory']) / 'fast' / (
            'horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    horizon = [binary, '--resources', lab.RESOURCES.resolve(), '--dedicated', '--nogui']
    net.PLAYERS = [PLAYER]
    net.PROPERTIES.update({'force-gamemode': 'false'})
    world = generate(vanilla, out)
    pack(world)
    seen = {side: run(side, command, client, out, world)
            for side, command in [('vanilla', vanilla), ('horizon', horizon)]}
    lab.write_json(out / 'report.json', seen)
    # Admission: on vanilla the player was falling before it moved up, and the move up ended that fall.
    falling = seen['vanilla']['down']
    assert falling['fall_distance'] == '1.0d' and not any(falling['advancements'].values()), falling
    risen = seen['vanilla']['up']
    assert risen['fall_distance'] == '0.0d', risen
    assert risen['advancements'] == {'any': True, 'short': True, 'five': False}, risen
    differences = [label for label in seen['vanilla'] if seen['vanilla'][label] != seen['horizon'][label]]
    for label in differences:
        print(f"difference after {label}: vanilla {seen['vanilla'][label]}, horizon {seen['horizon'][label]}",
              flush=True)
    print(f"{len(seen['vanilla']) - len(differences)}/{len(seen['vanilla'])} observations match", flush=True)
    if differences:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
