#!/usr/bin/env python3
"""Record official server duplicate-UUID passenger lifecycle, without a Rust build.

Uses the dedicated-server parity console transport and verified official jars.
Output is private vanilla data; never commit the output directory.
"""
import argparse
import json
import os
from pathlib import Path
import re
import time

import lab
import test_rust_parity as base
from test_dedicated_server_parity import Server, free_port, game_time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True, help='new private output directory')
    parser.add_argument('--cases', nargs='+', choices=('root_child', 'siblings', 'leaf', 'loaded'),
                        default=('root_child', 'siblings', 'leaf', 'loaded'))
    parser.add_argument('--root-type', choices=('pig', 'oak_boat', 'minecart'), default='pig')
    parser.add_argument('--action', choices=('all', 'kill', 'travel', 'same_dimension'),
                        default='all', help='observe one lifecycle without dismounting first')
    parser.add_argument('--final-restart', action='store_true',
                        help='save/restart again after the chosen lifecycle action')
    parser.add_argument('--cycle-chunk', action='store_true',
                        help='probe hidden-section UUID ownership after kill/travel (root_child only)')
    parser.add_argument('--raise-root-before-kill', action='store_true',
                        help='load root Pos at y 210 without moving its frozen passengers')
    parser.add_argument('--dismount-after-travel', choices=('grandchild', 'rejected'),
                        help='dismount this destination descendant after travel, then tick')
    args = parser.parse_args()
    if args.dismount_after_travel and args.action != 'travel':
        parser.error('--dismount-after-travel requires --action travel')
    if args.raise_root_before_kill and args.action != 'kill':
        parser.error('--raise-root-before-kill requires --action kill')
    if args.cycle_chunk and (args.cases != ['root_child'] or args.action not in ('kill', 'travel')):
        parser.error('--cycle-chunk requires --cases root_child and --action kill or travel')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    lock, jars, libraries = base.verify_artifacts()
    command = ['java', '-Xmx2g', '-cp', os.pathsep.join(map(str, [jars['server'], *libraries])),
               'net.minecraft.server.Main', 'nogui']
    world = out / 'server'
    world.mkdir()
    (world / 'eula.txt').write_text('eula=true\n')
    (world / 'server.properties').write_text(
        f'server-port={free_port()}\nonline-mode=false\nlevel-type=minecraft:flat\n'
        'generate-structures=false\nview-distance=2\nsimulation-distance=2\n'
        'pause-when-empty-seconds=-1\nmax-tick-time=-1\n')
    records = []
    serial = 0
    server = None

    def reply(text):
        nonlocal serial
        serial += 1
        marker = f'uuid_observation_end_{serial}'
        start = len(server.lines)
        server.console(text)
        server.console(f'say {marker}')
        server.wait_for(re.escape(f'[Server] {marker}') + '$')
        lines = [line for line in server.lines[start:] if marker not in line]
        records.append({'command': text, 'lines': lines})
        (out / 'records.json').write_text(json.dumps(records, indent=2))
        return lines

    def advance(ticks):
        before = game_time(server)
        reply(f'tick step {ticks}')
        deadline = time.monotonic() + 180
        while game_time(server) < before + ticks:
            assert time.monotonic() < deadline, 'tick step stalled'

    def observe(dimension=None):
        prefix = f'execute in minecraft:{dimension} run ' if dimension else ''
        for tag in ('root', 'first', 'rejected', 'grandchild', 'leaf'):
            reply(f'{prefix}data get entity @e[tag={tag},distance=0..,limit=1]')

    def start(label):
        nonlocal server
        server = Server(command, world, label)
        server.wait_for(r'Done \(')
        reply('tick freeze')
        reply('forceload add 0 0')
        # Blocking on the console command keeps all observations in server order.
        for _ in range(300):
            if any('Test passed' in line for line in reply('execute if loaded 0 200 0')):
                break
        else:
            raise AssertionError('summon chunk not loaded')

    def checkpoint(label, dimension=None):
        prefix = f'execute in minecraft:{dimension} run ' if dimension else ''
        if args.cycle_chunk:
            records.append({'checkpoint': f'{label}_before_unload'})
            reply('setworldspawn 1024 200 1024')
            reply(prefix + 'forceload add 1600 1600')
            reply(prefix + 'forceload remove 0 0')
            for _ in range(12):
                advance(100)
                if not any('Test passed' in line for line in reply(prefix + 'execute if loaded 0 200 0')):
                    break
            else:
                raise AssertionError('old chunk did not unload')
            records.append({'checkpoint': f'{label}_hidden'})
            observe(dimension)
            # u is the current case's UUID namespace; only root_child supports this probe.
            reply(prefix + 'summon pig 1601 200 1601 ' + entity('uuid_probe', u + 2))
            reply(prefix + 'forceload add 0 0')
            for _ in range(300):
                if any('Test passed' in line for line in reply(prefix + 'execute if loaded 0 200 0')):
                    break
            else:
                raise AssertionError('old chunk did not reload')
            records.append({'checkpoint': f'{label}_visible_again'})
            observe(dimension)
            reply(prefix + 'data get entity @e[tag=uuid_probe,distance=0..,limit=1]')
        if not args.final_restart:
            return
        reply('save-all flush')
        server.stop()
        start(label)
        records.append({'checkpoint': label})
        observe(dimension)
        if dimension:
            observe()

    def entity(tag, uuid, passengers=''):
        return ('{id:"minecraft:pig",UUID:[I;0,0,0,' + str(uuid) + '],Tags:["' + tag
                + '"],NoAI:1b,NoGravity:1b,Age:-1000'
                + (',Passengers:[' + passengers + ']' if passengers else '') + '}')

    try:
        start('initial')
        reply('gamerule minecraft:spawn_mobs false')
        # Each case gets distinct UUIDs so cleanup cannot mask admission behavior.
        for number, kind in enumerate(args.cases, 1):
            records.append({'case': kind})
            reply('kill @e[type=minecraft:pig]')
            u = number * 10
            if kind == 'root_child':
                passengers = entity('rejected', u, entity('grandchild', u + 2))
            elif kind == 'siblings':
                passengers = entity('first', u + 1) + ',' + entity('rejected', u + 1, entity('grandchild', u + 2))
            elif kind == 'leaf':
                passengers = entity('first', u + 1, entity('leaf', u + 1))
            else:
                reply('summon pig 2 200 2 ' + entity('first', u + 1))
                passengers = entity('rejected', u + 1, entity('grandchild', u + 2))
            reply(f'summon {args.root_type} 1 200 1 ' + entity('root', u, passengers))
            observe()
            advance(5)
            observe()
            reply('save-all flush')
            server.stop()
            start(f'reload_{kind}')
            observe()
            if args.action == 'kill':
                if args.raise_root_before_kill:
                    reply('data merge entity @e[tag=root,limit=1] {Pos:[1.0d,210.0d,1.0d]}')
                    observe()
                reply('kill @e[tag=root,distance=0..]')
                observe()
                advance(25)
                observe()
                checkpoint(f'after_kill_{kind}')
                continue
            if args.action == 'same_dimension':
                reply('teleport @e[tag=root,distance=0..] 4 200 4')
                observe()
                advance(5)
                observe()
                checkpoint(f'after_same_dimension_{kind}')
                continue
            if args.action == 'all':
                reply('execute as @e[tag=grandchild] run ride @s dismount')
                advance(5)
                observe()
            reply('execute in minecraft:the_nether run forceload add 0 0')
            for _ in range(300):
                if any('Test passed' in line for line in reply(
                        'execute in minecraft:the_nether if loaded 0 200 0')):
                    break
            else:
                raise AssertionError('destination chunk not loaded')
            reply('execute as @e[tag=root] in minecraft:the_nether run teleport @s 1 200 1')
            observe('the_nether')
            observe()
            advance(5)
            observe('the_nether')
            if args.dismount_after_travel:
                reply(f'execute in minecraft:the_nether as @e[tag={args.dismount_after_travel},'
                      'distance=0..] run ride @s dismount')
                observe('the_nether')
                advance(5)
                observe('the_nether')
            checkpoint(f'after_travel_{kind}', 'the_nether')
            reply('execute in minecraft:the_nether run kill @e[tag=root,distance=0..]')
            reply('kill @e[tag=root]')
            observe()
        (out / 'version.json').write_text(json.dumps(lock['minecraft'], indent=2))
    finally:
        if server is not None:
            server.stop()


if __name__ == '__main__':
    main()
