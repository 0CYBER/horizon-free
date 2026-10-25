#!/usr/bin/env python3
"""What a player holds on the cursor when they die, on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival and keep_inventory off. LocatorBarClient (a player made
of vanilla's protocol code) is given seven diamonds, picks them up onto its inventory menu's cursor (a PICKUP click)
ten blocks from the spawn, runs /kill on itself, waits on the death screen and respawns (PERFORM_RESPAWN). Twenty
ticks after dying vanilla removes the dead player (LivingEntity.tickDeath), and Player.remove's
inventoryMenu.removed drops the cursor's stack where it died. Alice then asks whether an item lies around the death
site; the reply, and the inventory slots, respawns and teleports she is sent from the death on, are compared
exactly.
"""
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

try:
    from . import lab, test_rust_parity as base, test_locator_bar_parity as locator
    from .test_dedicated_server_parity import Server
except ImportError:
    import lab
    import test_rust_parity as base
    import test_locator_bar_parity as locator
    from test_dedicated_server_parity import Server

NAME = 'cursor-death-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')
SCRIPT = [
    ('setup', ['cmd Alice gamerule keep_inventory false', 'cmd Alice clear @s',
               'cmd Alice give @s minecraft:diamond 7', 'cmd Alice tp @s 10.5 -60 0.5']),
    # Hotbar slot 0 is the inventory menu's slot 36.
    ('pickup', ['pickup Alice 36']),
    ('death', ['cmd Alice kill @s']),
    ('respawn', ['respawn Alice']),
    ('check', ['chat Alice on',
               'cmd Alice execute if entity @e[type=minecraft:item,x=10.5,y=-60,z=0.5,distance=..4]']),
]
STEP_SECONDS = 4.0
SHOWN = ('slot', 'inventory', 'respawn', 'teleport', 'chat')


def run_side(label, server_command, client_command, build, world):
    directory = build / label
    port = locator.server_dir(directory, world)
    out = build / f'{label}.txt'
    out.unlink(missing_ok=True)
    server = Server(server_command, directory, label)
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-client.log').open('w') as log:
            client = subprocess.Popen([*map(str, client_command), '127.0.0.1', str(port), str(out)],
                                      cwd=lab.ROOT, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT,
                                      text=True)

            def tell(line):
                client.stdin.write(line + '\n')
                client.stdin.flush()
            try:
                tell('players on')
                tell('state on')
                tell('step joins')
                tell('join Alice')
                server.wait_for(r'Alice joined the game', timeout=120)
                time.sleep(STEP_SECONDS)
                for step, actions in SCRIPT:
                    tell(f'step {step}')
                    for action in actions:
                        tell(action.replace('chat Alice', 'chat'))
                    time.sleep(STEP_SECONDS)
                tell('end')
                client.wait(timeout=60)
            finally:
                if client.poll() is None:
                    client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
    return out.read_text(encoding='utf-8').splitlines()


def shown(lines):
    """Alice's slots, respawns, teleports and chat from the death on."""
    steps = {'step=death', 'step=respawn', 'step=check'}
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3 and parts[1] in steps
            and parts[2] in SHOWN]


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
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
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.LocatorBarClient']

    world = locator.generate_world(vanilla_server, build)
    vanilla_all = run_side('vanilla', vanilla_server, client, build, world)
    horizon_all = run_side('horizon', horizon_server, client, build, world)
    errors = [l for l in vanilla_all + horizon_all if ' error ' in f' {l} ' or 'disconnected' in l]
    vanilla, horizon = shown(vanilla_all), shown(horizon_all)
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:6000])
    if not any(l.startswith('Alice step=check chat') for l in vanilla):
        raise AssertionError(f'vanilla answered no check command: the script did not run (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='A player dying with a stack on the inventory menu cursor (keep_inventory off) and respawning on a '
              'dedicated server: the stack is dropped where it died, not kept.'))
    print(f'cursor death: {len(vanilla)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
