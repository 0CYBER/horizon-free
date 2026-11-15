#!/usr/bin/env python3
"""An entity that starts ticking goes to the end of the tick order. Dedicated servers.

`ServerLevel.entityTickList` holds only the entities in entity-ticking sections. One in a chunk that does not tick
entities is out of it (`PersistentEntitySectionManager.stopTicking`, `EntityCallbacks.onTickingEnd`); a move that
brings it into a ticking chunk (`Callback.onMove` -> `updateStatus` -> `startTicking`) adds it again, at the end
(`EntityTickList.add` on an `Int2ObjectLinkedOpenHashMap`). So it ticks after every entity already ticking, even
ones that joined the level after it.

A vanilla dedicated server and Horizon's each load a copy of one flat world vanilla generated first. Ticking is
frozen; only chunks (10, 0) and (10, 4) are forced, so chunk (11, 0) (x 176 and on) does not tick its entities.
In each lane a primed TNT without gravity, `late`, is summoned first, then a TNT `blast` with a fuse of 2, 3 blocks
from where `late` will stand. In the test lane `late` starts in chunk (11, 0) and is teleported next to `blast`
after the first step; in the control lane it is there from the start (the teleport is to where it already is).
At the second step `blast` explodes: in the test lane vanilla ticks it before `late`, so `late` moves with the push
in that step; in the control lane `late` ticks first and moves a step later. The servers are stepped; after every
step each TNT's exact Pos and Motion are read with `data get`. The transcripts must match vanilla's (the oracle)
line for line, and vanilla's must show the two orders.
"""
import re
import sys
import time

try:
    from . import test_locator_bar_parity as locator
    from .test_dedicated_server_parity import Server
except ImportError:
    import test_locator_bar_parity as locator
    from test_dedicated_server_parity import Server

NAME = 'entity-restart-tick-order-parity'
TICKS = 4
Y = -50
# lane, its z, where `late` starts
LANES = [('test', 8, 180.5), ('control', 72, 173.5)]
FORCELOAD = ['forceload add 160 0 175 15', 'forceload add 160 64 175 79']
NUMBER = re.compile(r'(-?[0-9][0-9.E-]*)[df]')


def tnt(tag, x, z, fuse):
    return (f'summon minecraft:tnt {x} {Y}.0 {z}.5 '
            f'{{fuse:{fuse}s,NoGravity:1b,Motion:[0.0d,0.0d,0.0d],Tags:["{tag}"]}}')


# Every `late` joins before every `blast`.
ENTITIES = [*[tnt(f'{lane}_late', x, z, 80) for lane, z, x in LANES],
            *[tnt(f'{lane}_blast', 170.5, z, 2) for lane, z, _x in LANES]]
AFTER_FIRST_STEP = [f'tp @e[tag={lane}_late,limit=1] 173.5 {Y}.0 {z}.5' for lane, z, _x in LANES]
TAGS = [f'{lane}_{role}' for lane, *_rest in LANES for role in ('late', 'blast')]


def run(label, command, world, build):
    directory = build / label
    locator.server_dir(directory, world)
    server = Server(command, directory, label)
    lines = []
    try:
        server.wait_for(r'Done \(', timeout=600)
        server.console('tick freeze')
        for command_line in FORCELOAD:
            server.console(command_line)
        server.console('say READY')
        server.wait_for(r'READY', timeout=120)
        time.sleep(3)
        for command_line in ENTITIES:
            server.console(command_line)
        time.sleep(1)
        for tick in range(TICKS + 1):
            if tick:
                server.console('tick step 1')
                time.sleep(1.0)
            if tick == 1:
                for command_line in AFTER_FIRST_STEP:
                    server.console(command_line)
                time.sleep(0.5)
            start = len(server.lines)
            for tag in TAGS:
                for path in ('Pos', 'Motion'):
                    server.console(f'data get entity @e[tag={tag},limit=1] {path}')
            server.console(f'say END {tick}')
            server.wait_for(rf'END {tick}\b', timeout=120)
            answers = [l.split('INFO]: ', 1)[-1] for l in server.lines[start:]
                       if 'entity data:' in l or 'No entity was found' in l or 'Found no' in l]
            for i, tag in enumerate(TAGS):
                pos, motion = answers[2 * i:2 * i + 2] if len(answers) >= 2 * i + 2 else ('', '')
                if 'entity data:' not in pos:
                    lines.append(f'tick {tick} {tag} gone')
                else:
                    lines.append(f'tick {tick} {tag} pos {" ".join(NUMBER.findall(pos))}'
                                 f' motion {" ".join(NUMBER.findall(motion))}')
    finally:
        server.stop()
    return lines


def proves(vanilla):
    """Why vanilla's transcript does not show what this harness is for, or None."""
    def pos(tick, tag):
        line = next(l for l in vanilla if l.startswith(f'tick {tick} {tag} '))
        return line.split(' pos ')[1].split(' motion ')[0] if ' pos ' in line else None
    if pos(2, 'test_blast') is not None or pos(2, 'control_blast') is not None:
        return 'a blast TNT did not explode at the second step'
    if pos(2, 'test_late') == pos(1, 'test_late'):
        return 'the teleported TNT did not move with the push in the step it was pushed'
    if pos(2, 'control_late') != pos(1, 'control_late') or pos(3, 'control_late') == pos(2, 'control_late'):
        return 'the TNT that ticked all along did not tick before the blast'
    return None


def main():
    lock, build, vanilla_server, horizon_server, _client = locator.sides(NAME)
    world = locator.generate_world(vanilla_server, build)
    vanilla = run('vanilla', vanilla_server, world, build)
    (build / 'vanilla.txt').write_text('\n'.join(vanilla) + '\n', encoding='utf-8')
    why = proves(vanilla)
    if why:
        raise AssertionError(f'vanilla is no proof: {why}:\n' + '\n'.join(vanilla))
    horizon = run('horizon', horizon_server, world, build)
    (build / 'horizon.txt').write_text('\n'.join(horizon) + '\n', encoding='utf-8')
    if vanilla != horizon:
        differ = [f'- {v}\n+ {h}' for v, h in zip(vanilla, horizon) if v != h]
        raise AssertionError(f'{len(differ)} lines differ from vanilla:\n' + '\n'.join(differ[:24]))
    print(f'entity restart tick order: {len(vanilla)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
