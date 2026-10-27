#!/usr/bin/env python3
"""The dragon fight on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat
world vanilla generated first, with the same server.properties. A vanilla-codec client (VanillaProtocolClient in
its entity mode) joins, and once its chunks are in, the console sends it to the End, over the exit portal. Nobody
else is in the End: the fight must start from that player alone (`EnderDragonFight.tick` finds it among the
level's players, `createNewDragon` spawns the dragon over the podium) and the dragon must be tracked to the client
(`ServerEntity`'s pairing: ClientboundAddEntityPacket, the synched data, the attribute instances).

The dragon's flight is random (its yaw, its holding-pattern turns), so its position, rotations and motion are
left out; its type, spawn data, which synched data entries it was paired with and sent (and their serializers)
and its attribute instances must match vanilla's exactly.
"""
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server
    from .test_locator_bar_parity import generate_world, server_dir
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server
    from test_locator_bar_parity import generate_world, server_dir

NAME = 'server-dragon-parity'
PLAYER = 'Tester'
# Over the exit portal, well within 192 blocks of (0, 128, 0) and the dragon's holding pattern.
TO_THE_END = f'execute in minecraft:the_end run tp {PLAYER} 0.5 80 0.5'


def run_side(label, server_command, client_command, build, world):
    directory = build / label
    port = server_dir(directory, world)
    out = build / f'{label}-client.txt'
    out.unlink(missing_ok=True)
    server = Server(server_command, directory, label)
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-client.log').open('w') as log:
            client = subprocess.Popen([*map(str, client_command), '127.0.0.1', str(port), PLAYER, str(out), 'entities'],
                                      cwd=lab.ROOT, stdout=log, stderr=subprocess.STDOUT, text=True)
            try:
                server.wait_for(rf'<{PLAYER}> summon the entities', timeout=180)
                server.console(TO_THE_END)
                client.wait(timeout=120)
            finally:
                if client.poll() is None:
                    client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
    lines = out.read_text(encoding='utf-8').splitlines()
    (build / f'{label}.txt').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    return lines


def entities(lines, kind):
    return [l for l in lines if l.startswith(f'play-entities entity {kind} ')]


def dragon_summary(line):
    """What of a tracked dragon does not depend on its random flight: its spawn data, the synched data entries
    (id and serializer) it was paired with and sent, and its attribute instances."""
    fields = dict(f.split(' ', 1) for f in line.split(' | ')[1:] if ' ' in f)
    data = re.search(r' data (-?\d+) ', fields.get('add', ''))
    pairing = sorted(int(e.split('=', 1)[0]) for e in fields.get('pairing', '').split() if '=' in e)
    sent = sorted((int(k), v.split(' ', 1)[0]) for k, v in re.findall(r'(\d+)=(\d+ [^,}]*)', fields.get('data', '')))
    return {
        'spawn data': data.group(1) if data else None,
        'paired entries': pairing,
        'entries (id, serializer)': sent,
        'attributes': fields.get('attributes'),
        'equipment': fields.get('equipment'),
        'passengers': fields.get('passengers'),
    }


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaProtocolClient.java'
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                   cwd=lab.ROOT, check=True, capture_output=True)
    # SERVER_DRAGON_SIDES=vanilla (or horizon) runs one side and reads the other's last transcript.
    sides = [s for s in os.environ.get('SERVER_DRAGON_SIDES', 'vanilla,horizon').split(',') if s]
    if 'horizon' in sides:
        subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                       check=True, capture_output=True)
    import json
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-Duser.timezone=UTC', '-cp', str(classes) + os.pathsep + dependencies,
              'dev.horizonlab.parity.VanillaProtocolClient']

    world = build / 'generate' / 'world'
    if not world.exists() or set(sides) == {'vanilla', 'horizon'}:
        world = generate_world(vanilla_server, build)
    results = {}
    for label, command in [('vanilla', vanilla_server), ('horizon', horizon_server)]:
        if label in sides:
            results[label] = run_side(label, command, client, build, world)
        else:
            results[label] = (build / f'{label}.txt').read_text(encoding='utf-8').splitlines()
    problems = []
    summaries = {}
    for label, lines in results.items():
        errors = [l for l in lines if l.startswith('error')]
        problems += [f'{label}: {e}' for e in errors[:5]]
        dragons = entities(lines, 'minecraft:ender_dragon')
        if not dragons:
            problems.append(f'{label}: no dragon was tracked to the player in the End')
            continue
        summaries[label] = dragon_summary(dragons[0])
    if not problems and summaries['vanilla'] != summaries['horizon']:
        problems.append(f'the dragons differ:\n  vanilla {summaries["vanilla"]}\n  horizon {summaries["horizon"]}')
    if problems:
        raise AssertionError(f'see {build}/vanilla.txt and horizon.txt:\n' + '\n'.join(problems))
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        dragon=summaries['horizon'], output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='A lone player sent to the End of a dedicated server: the fight starts from it, the dragon spawns '
              'and is tracked to its vanilla-codec client (spawn data, synched data entries, attributes).'))
    print(f'server dragon: the fight started, dragon tracked as vanilla does ({summaries["horizon"]})')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
