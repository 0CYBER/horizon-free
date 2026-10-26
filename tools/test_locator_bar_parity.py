#!/usr/bin/env python3
"""The locator bar on a dedicated server, against vanilla's, seen by vanilla clients.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat
world vanilla generated first (players appear at its spawn), with the same server.properties (offline,
creative, view distance 4) and two operators. LocatorBarClient, two
players made of vanilla's protocol code (any byte vanilla would not read is an error), joins Alice and then
Bob, who fly as creative players do (a player floating without flying falls on the server every tick), and
records every ClientboundTrackedWaypointPacket each of them receives. Bob then:

- teleports one block, three blocks, out of Alice's view, one chunk on, really far (an azimuth) and turns
  around her, and back;
- turns invisible and visible, joins a red team (by the console), has it turned blue, leaves it;
- crouches and stands up, wears a carved pumpkin and takes it off;
- turns spectator and moves, then creative again;

the console turns the `locator_bar` game rule off and on, and Bob leaves. The waypoint packets each player
received in each step are compared exactly: operation, player, style, colour and the block, chunk or angle.
"""
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port

NAME = 'locator-bar-parity'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-seed': '12345',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
    'max-players': '5',
    'pause-when-empty-seconds': '-1',
}
PLAYERS = ['Alice', 'Bob']

# (label, [what the console does], [what the players do]); every step then waits for its packets.
SCRIPT = [
    ('alice-joins', [], ['join Alice', 'fly Alice on', 'cmd Alice tp @s 0.5 100 0.5']),
    ('bob-joins', [], ['join Bob', 'fly Bob on', 'cmd Bob tp @s 2.5 100 0.5']),
    ('near', [], ['cmd Bob tp @s 5.5 100 0.5']),
    ('one-block', [], ['cmd Bob tp @s 6.5 100 0.5']),
    ('three-blocks', [], ['cmd Bob tp @s 9.5 100 0.5']),
    ('out-of-view', [], ['cmd Bob tp @s 100.5 100 0.5']),
    ('next-chunk', [], ['cmd Bob tp @s 116.5 100 0.5']),
    ('really-far', [], ['cmd Bob tp @s 1000.5 100 0.5']),
    ('turn-around', [], ['cmd Bob tp @s 1000.5 100 300.5']),
    ('half-degree', [], ['cmd Bob tp @s 1000.5 100 302.5']),
    ('invisible', [], ['cmd Bob effect give @s minecraft:invisibility infinite 0 true']),
    ('visible', [], ['cmd Bob effect clear @s']),
    ('back-near', [], ['cmd Bob tp @s 3.5 100 0.5']),
    ('red-team', ['team add red', 'team modify red color red', 'team join red Bob'], []),
    ('blue-team', ['team modify red color blue'], []),
    ('no-team', ['team leave Bob'], []),
    # Crouching needs the ground: a flying player who holds shift goes down instead.
    ('landed', [], ['cmd Bob tp @s 3.5 -60 0.5', 'fly Bob off']),
    ('crouch', [], ['shift Bob on']),
    ('stand', [], ['shift Bob off']),
    ('flying-again', [], ['fly Bob on', 'cmd Bob tp @s 3.5 100 0.5']),
    ('pumpkin', [], ['cmd Bob item replace entity @s armor.head with minecraft:carved_pumpkin']),
    ('no-pumpkin', [], ['cmd Bob item replace entity @s armor.head with minecraft:air']),
    ('rule-off', ['gamerule locator_bar false'], []),
    ('moves-without', [], ['cmd Bob tp @s 4.5 100 0.5']),
    ('rule-on', ['gamerule locator_bar true'], []),
    ('spectator', [], ['cmd Bob gamemode spectator', 'cmd Bob tp @s 5.5 100 0.5']),
    ('creative', [], ['cmd Bob gamemode creative', 'cmd Bob tp @s 6.5 100 0.5']),
    ('bob-leaves', [], ['quit Bob']),
]
STEP_SECONDS = 2.0
# Vanilla's server moves a player who just joined on their own first ticks (a tiny step and back), which tracks
# them where they spawned; Horizon does not simulate its guests. These steps compare what the bar shows at
# their end (Bob has moved by then) rather than every packet.
SETTLING_STEPS = {'alice-joins', 'bob-joins'}


def shown(lines):
    """The transcript with each settling step reduced to what each player's bar shows at its end."""
    out, state = [], {}

    def flush(step):
        for (receiver, source), rest in sorted(state.items()):
            out.append(f'{receiver} step={step} shows {source} {rest}')
        state.clear()

    current = None
    for line in lines:
        receiver, step, *packet = line.split(' ')
        step = step.removeprefix('step=')
        if current in SETTLING_STEPS and step != current:
            flush(current)
        current = step
        if step not in SETTLING_STEPS or len(packet) < 3:
            out.append(line)
        elif packet[0] == 'untrack':
            state.pop((receiver, packet[1]), None)
        else:
            state[(receiver, packet[1])] = ' '.join(packet[2:])
    if current in SETTLING_STEPS:
        flush(current)
    return out


def ops_json():
    """Both players at level 4, by their offline UUIDs."""
    import hashlib
    import json
    import uuid as uuidlib
    entries = []
    for name in PLAYERS:
        digest = bytearray(hashlib.md5(f'OfflinePlayer:{name}'.encode()).digest())
        digest[6] = (digest[6] & 0x0F) | 0x30
        digest[8] = (digest[8] & 0x3F) | 0x80
        entries.append({'uuid': str(uuidlib.UUID(bytes=bytes(digest))), 'name': name, 'level': 4,
                        'bypassesPlayerLimit': False})
    return json.dumps(entries, indent=2)


def server_dir(directory, world=None):
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    port = free_port()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    (directory / 'ops.json').write_text(ops_json())
    if world is not None:
        shutil.copytree(world, directory / 'world')
        (directory / 'world' / 'session.lock').unlink(missing_ok=True)
    return port


def generate_world(server_command, build):
    """The flat world both servers load, made once by vanilla: players appear at its spawn itself
    (respawn_radius 0), on the ground at 0 -60 0, and the chunks around it are saved."""
    directory = build / 'generate'
    server_dir(directory)
    server = Server(server_command, directory, 'generate')
    try:
        server.wait_for(r'Done \(')
        for command, reply in [('gamerule respawn_radius 0', 'respawn_radius'),
                               ('setworldspawn 0 -60 0', 'Set the world spawn'),
                               ('forceload add -96 -96 96 96', 'force loaded'),
                               ('save-all flush', 'Saved the game')]:
            server.console(command)
            server.wait_for(reply, timeout=120)
        server.console('forceload remove all')
    finally:
        server.stop()
    return directory / 'world'


def run_side(label, server_command, client_command, build, world):
    directory = build / label
    port = server_dir(directory, world)
    out = build / f'{label}.txt'
    out.unlink(missing_ok=True)
    server = Server(server_command, directory, label)
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-client.log').open('w') as log:
            client = subprocess.Popen([*map(str, client_command), '127.0.0.1', str(port), str(out)],
                                      cwd=lab.ROOT, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT,
                                      text=True)
            try:
                for step, console, actions in SCRIPT:
                    client.stdin.write(f'step {step}\n')
                    client.stdin.flush()
                    for command in console:
                        server.console(command)
                    for action in actions:
                        client.stdin.write(action + '\n')
                        client.stdin.flush()
                        if action.startswith('join'):
                            # Joining takes the world's first chunks: wait for the server to place the player.
                            server.wait_for(rf'{action.split()[1]} joined the game', timeout=120)
                    time.sleep(STEP_SECONDS)
                client.stdin.write('end\n')
                client.stdin.flush()
                client.wait(timeout=60)
            finally:
                if client.poll() is None:
                    client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
    return out.read_text(encoding='utf-8').splitlines()


def sides(name):
    """The vanilla and Horizon dedicated servers and the LocatorBarClient that joins them, built for harness
    `name`: (lock, build directory, vanilla server, Horizon server, client) commands."""
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / name
    build.mkdir(parents=True, exist_ok=True)
    (build / 'report.json').unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/LocatorBarClient.java'
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                   cwd=lab.ROOT, check=True, capture_output=True)
    subprocess.run(['cargo', 'build', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    import json
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.LocatorBarClient']
    return lock, build, vanilla_server, horizon_server, client


def one_player(name, script, shown, check, scope, step_seconds=4.0):
    """A harness of one player, Alice: on each server she joins, then runs `script`, a list of (step, client
    actions) `step_seconds` apart. The lines `shown(transcript)` keeps must match vanilla's exactly, and
    `check(vanilla's shown lines, vanilla's transcript)` returns why vanilla's run is no proof (the script did
    not run), or None."""
    lock, build, vanilla_server, horizon_server, client = sides(name)

    def run(label, server_command, world):
        directory = build / label
        port = server_dir(directory, world)
        out = build / f'{label}.txt'
        out.unlink(missing_ok=True)
        server = Server(server_command, directory, label)
        try:
            server.wait_for(r'Done \(')
            with (build / f'{label}-client.log').open('w') as log:
                process = subprocess.Popen([*map(str, client), '127.0.0.1', str(port), str(out)], cwd=lab.ROOT,
                                           stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True)

                def tell(line):
                    process.stdin.write(line + '\n')
                    process.stdin.flush()
                try:
                    for line in ['players on', 'state on', 'step joins', 'join Alice']:
                        tell(line)
                    server.wait_for(r'Alice joined the game', timeout=120)
                    time.sleep(step_seconds)
                    for step, actions in script:
                        tell(f'step {step}')
                        for action in actions:
                            tell(action)
                        time.sleep(step_seconds)
                    tell('end')
                    process.wait(timeout=60)
                finally:
                    if process.poll() is None:
                        process.kill()
        finally:
            server.stop()
        if not out.exists():
            raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
        return out.read_text(encoding='utf-8').splitlines()

    world = generate_world(vanilla_server, build)
    vanilla_all = run('vanilla', vanilla_server, world)
    horizon_all = run('horizon', horizon_server, world)
    errors = [l for l in vanilla_all + horizon_all if ' error ' in f' {l} ' or 'disconnected' in l]
    vanilla, horizon = shown(vanilla_all), shown(horizon_all)
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:6000])
    if (why := check(vanilla, vanilla_all)) is not None:
        raise AssertionError(f'{why}: the script did not run (see {build})')
    lab.write_json(build / 'report.json', dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'), scope=scope))
    return vanilla


def main():
    lock, build, vanilla_server, horizon_server, client = sides(NAME)
    report = build / 'report.json'
    world = generate_world(vanilla_server, build)
    vanilla = run_side('vanilla', vanilla_server, client, build, world)
    horizon = run_side('horizon', horizon_server, client, build, world)
    errors = [l for l in vanilla + horizon if ' error ' in f' {l} ' or 'disconnected' in l]
    vanilla, horizon = shown(vanilla), shown(horizon)
    # Each player's packets in their order; which player's line comes first in a step doesn't matter.
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:6000])
    if len(vanilla) < 20:
        raise AssertionError(f'only {len(vanilla)} waypoint packets: the script did not run (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        packets=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='Locator bar waypoints (ClientboundTrackedWaypointPacket) two vanilla-codec players receive on a '
              'dedicated server: block, chunk and azimuth connections as one moves, invisibility, team colours, '
              'crouching, a carved pumpkin, the locator_bar game rule, spectator mode and leaving.'))
    print(f'locator bar: {len(vanilla)} waypoint packets identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
