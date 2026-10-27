#!/usr/bin/env python3
"""The End of a dedicated server nobody has entered, force-loaded from the console, against vanilla's.

A vanilla dedicated server and Horizon's each open a copy of one world vanilla generated (its End not made yet).
With no player online, the console force-loads the End's main island and its arena. A FORCED ticket keeps its level active
(`ServerChunkCache.hasActiveTickets`, `TicketType.FORCED`), so the chunks are generated, loaded and ticked
without a player in the dimension. With nobody in the fight, `EnderDragonFight.tick` neither scans the state nor
places the exit portal: no bedrock and no torches around 0, 0. Both servers must answer the same.
"""
import os
import re
import subprocess
import sys
import time

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, wait_ticks
    from .test_locator_bar_parity import generate_world, server_dir
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, wait_ticks
    from test_locator_bar_parity import generate_world, server_dir

NAME = 'forced-end-parity'
END = 'execute in minecraft:the_end'
QUERIES = [
    f'{END} run fill -3 0 -3 3 255 3 minecraft:air replace minecraft:bedrock',
    f'{END} run fill -3 0 -3 3 255 3 minecraft:air replace minecraft:wall_torch',
]


def answer(server, command, n):
    start = len(server.lines)
    server.console(command)
    server.console(f'say done {n}')
    server.wait_for(rf'\[Server\] done {n}$', timeout=120)
    replies = [l.split(']: ', 1)[-1] for l in server.lines[start:] if 'done ' not in l]
    return ' / '.join(r for r in replies if r) or '(no reply)'


def run_side(label, command, build, world):
    directory = build / label
    server_dir(directory, world)
    server = Server(command, directory, label)
    try:
        server.wait_for(r'Done \(')
        # The main island's arena (EnderDragonFight.isArenaLoaded: chunks -8..8), in pieces of at most 256 chunks.
        for area in ('-128 -128 143 -1', '-128 0 143 143'):
            server.console(f'{END} run forceload add {area}')
        deadline = time.time() + 600
        for poll in range(1_000_000):
            reply = answer(server, f'{END} if loaded 0 0 0', f'loaded {poll}')
            if 'passed' in reply:
                break
            if time.time() > deadline:
                raise AssertionError(f'{label}: the End centre never loaded')
            time.sleep(2)
        wait_ticks(server, 200)
        return {q: answer(server, q, i) for i, q in enumerate(QUERIES)}
    finally:
        server.stop()


def main():
    _, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['cargo', 'build', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    binary = lab.cargo_target() / 'release' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    world = generate_world(vanilla_server, build)
    vanilla = run_side('vanilla', vanilla_server, build, world)
    horizon = run_side('horizon', horizon_server, build, world)
    normal = lambda s: re.sub(r'\s+', ' ', s).strip()
    problems = [f'{q}\n  vanilla: {vanilla[q]}\n  horizon: {horizon[q]}'
                for q in QUERIES if normal(vanilla[q]) != normal(horizon[q])]
    if problems:
        raise AssertionError('the forced End differs:\n' + '\n'.join(problems))
    print(f'forced End: loaded and ticked with nobody in it, no exit portal, as vanilla ({vanilla[QUERIES[0]]})')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
