#!/usr/bin/env python3
"""Both players' ordinary walking activates a sculk sensor on the server.

Exercise Entity.move movement emission through real client input, alongside
sculk's deterministic vanilla oracle. Check the integrated-server and in-frame
hosts: the guest always uses the server-player path.
"""
import json
import math
import subprocess

import lab
import lan_e2e as lan


def until(game, observe, predicate, ticks=100):
    value = observe()
    for _ in range(ticks):
        if predicate(value):
            return value
        game.wait(1)
        value = observe()
    assert predicate(value), value
    return value


def check_walk(host, guest, out, role):
    player = host if role == 'host' else guest
    hx, y, hz = (math.floor(v) for v in lan.pos_of(host))
    x, z = hx + 6, hz + (-8 if role == 'host' else 8)
    sensor = (x + 3, y, z + 1)
    player('command', command='gamemode survival')
    host('command', command=f'fill {x - 2} {y - 1} {z - 2} {x + 5} {y - 1} {z + 5} minecraft:stone')
    host('command', command=f'fill {x - 2} {y} {z - 2} {x + 5} {y + 3} {z + 5} minecraft:air')
    start = (x + 0.5, y, z + 0.5)
    player('command', command=f'tp @s {start[0]} {start[1]} {start[2]} 0 0')
    until(player, lambda: lan.pos_of(player), lambda p: math.dist(p, start) < 0.1)
    player.raw('look', 0, 0)
    player.wait(2)
    host('command', command=f'setblock {sensor[0]} {sensor[1]} {sensor[2]} minecraft:sculk_sensor')
    phase = lambda game: game.raw('getblock', *sensor)['block']
    try:
        until(host, lambda: phase(host), lambda s: 'sculk_sensor_phase=inactive' in s)
        player.wait(10)
        assert 'sculk_sensor_phase=inactive' in phase(host), 'setup activated the sensor'
        player.raw('key', 'w', 'down')
        try:
            player.wait(15)
        finally:
            player.raw('key', 'w', 'up')
        moved = {'client': math.dist(lan.pos_of(player), start),
                 'server': math.dist(lan.server_player(host, player)['position'], start)}
        assert all(d > 1 for d in moved.values()), moved
        for viewer in (host, guest):
            until(viewer, lambda: phase(viewer), lambda s: 'sculk_sensor_phase=active' in s, ticks=85)
        return {'moved': moved, 'phase': phase(host)}
    finally:
        player.raw('key', 'w', 'up')
        host('command', command=f'fill {x - 2} {y} {z - 2} {x + 5} {y + 3} {z + 5} minecraft:air')


def main():
    subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client',
                    '--bin', 'horizon-client', '--bin', 'horizon-ctl'], cwd=lab.ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=lab.ROOT, text=True))
    ctl = lab.ROOT / metadata['target_directory'] / 'fast' / 'horizon-ctl'
    checks = ['sculk-walk-host', 'sculk-walk-guest']
    for role, name in zip(('host', 'guest'), checks):
        lan.CHECK_FNS[name] = lambda h, g, o, role=role: check_walk(h, g, o, role)
    # Use the runner's current display (xvfb on Linux), with fresh isolated homes.
    lan.game_env = lambda _screen, home: lab.game_env(
        HORIZON_HOME=home, HORIZON_TELEMETRY_URL='off')
    reports = {}
    for threaded in (True, False):
        lan.SERVER_THREAD = threaded
        mode = 'server-thread' if threaded else 'in-frame'
        out = lab.LOCAL / 'server-player-sculk-parity' / mode
        out.mkdir(parents=True, exist_ok=True)
        # lan.Game launches each client with lab.game_env and private scratch homes.
        reports[mode] = lan.run(ctl, out, lab.RESOURCES.resolve(), checks)
    lab.write_json(lab.LOCAL / 'server-player-sculk-parity' / 'report.json', reports)
    assert all(r['pass'] for mode in reports.values() for r in mode.values()), reports


if __name__ == '__main__':
    main()
