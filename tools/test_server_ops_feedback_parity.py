#!/usr/bin/env python3
"""Command feedback the operators get on a dedicated server (`[Server: ...]`), against vanilla's.

CommandSourceStack.broadcastToAdmins: a command's success that vanilla sends with `broadcast` goes, gray and
italic as `[name: message]`, to every other operator while `send_command_feedback` is on, including the
console's own commands when `broadcast-console-to-ops` is (DedicatedServer.shouldInformAdmins).

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat
world vanilla generated first, with the same server.properties and Alice and Bob as operators. LocatorBarClient
joins them both (players made of vanilla's protocol code) and records the system chat each receives while, step
by step, the console sets the time, asks for the difficulty (a query, told to nobody), whitelists a player,
deops Bob (who stops hearing of it) and ops him again, turns `send_command_feedback` off and on and saves, and
Alice runs a command herself (Bob hears of it as an operator). Then again with `broadcast-console-to-ops=false`.
Each player's chat in each step is compared exactly.
"""
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, game_time
    from .test_locator_bar_parity import Client, generate_world, server_dir
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, game_time
    from test_locator_bar_parity import Client, generate_world, server_dir

NAME = 'server-ops-feedback-parity'
# (label, what the console does, what the players do).
BROADCAST = [
    ('console-time', ['time set 1000'], []),
    ('console-query', ['difficulty'], []),
    ('console-whitelist', ['whitelist add Alice'], []),
    ('console-deop', ['deop Bob'], []),
    ('bob-not-op', ['time set 2000'], []),
    ('alice-bob-not-op', [], ['cmd Alice time set 3000']),
    ('console-op', ['op Bob'], []),
    ('alice-bob-op', [], ['cmd Alice time set 4000']),
    ('feedback-off', ['gamerule send_command_feedback false'], []),
    ('console-feedback-off', ['time set 5000'], []),
    ('feedback-on', ['gamerule send_command_feedback true'], []),
    ('console-save', ['save-all'], []),
]
QUIET = [
    ('console-time', ['time set 1000'], []),
    ('console-whitelist', ['whitelist add Alice'], []),
    ('alice', [], ['cmd Alice time set 3000']),
]
CASES = {'broadcast': ({}, BROADCAST), 'quiet': ({'broadcast-console-to-ops': 'false'}, QUIET)}
STEP_TICKS = 40
# A step ends by the server's clock (Client.wait): after a round trip of Alice's (Bob is deopped for a while, so
# his is not used). While send_command_feedback is off a command has no reply for her, so those steps wait for
# the ticks alone.
NO_SYNC = {'feedback-off', 'console-feedback-off'}
# Steps whose reply can take longer than a step: the next step waits for this console line first. A loaded machine
# took 2 s to save vanilla's world, and the transcript ended before "Saved the game".
WAIT_FOR = {'console-save': r'System chat: Saved the game'}


def run_side(label, case, server_command, client_command, build, world):
    properties, script = CASES[case]
    directory = build / label / case
    directory.parent.mkdir(parents=True, exist_ok=True)
    port = server_dir(directory, world)
    with (directory / 'server.properties').open('a', encoding='iso-8859-1') as f:
        f.writelines(f'{k}={v}\n' for k, v in properties.items())
    out = build / f'{label}-{case}.txt'
    out.unlink(missing_ok=True)
    server = Server(server_command, directory, f'{label}-{case}')
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-{case}-client.log').open('w') as log:
            client = Client(client_command, port, out, log)
            try:
                for name in ('Alice', 'Bob'):
                    client.tell(f'join {name}')
                    server.wait_for(rf'{name} joined the game', timeout=120)
                client.wait(server, ['Alice'], STEP_TICKS)
                client.tell('chat on')
                for step, console, actions in script:
                    client.tell(f'step {step}')
                    for command in console:
                        server.console(command)
                    game_time(server)  # the console runs its queue in order: these commands have run
                    for action in actions:
                        client.tell(action)
                    if step in WAIT_FOR:
                        server.wait_for(WAIT_FOR[step], timeout=120)
                    client.wait(server, [] if step in NO_SYNC else ['Alice'], STEP_TICKS)
                client.end()
            finally:
                client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label} {case}: the client wrote no transcript (see {build}/{label}-{case}-client.log)')
    lines = out.read_text(encoding='utf-8').splitlines()
    errors = [l for l in lines if ' error ' in f' {l} ' or 'disconnected' in l]
    return [f'{case} {l}' for l in lines if l.split(' ', 2)[2:3] and l.split(' ', 2)[2].startswith('chat ')], errors


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
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    import json
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.LocatorBarClient']

    world = generate_world(vanilla_server, build)
    sides = {'vanilla': ([], []), 'horizon': ([], [])}
    for case in CASES:
        for label, command in [('vanilla', vanilla_server), ('horizon', horizon_server)]:
            lines, errors = run_side(label, case, command, client, build, world)
            sides[label][0].extend(lines)
            sides[label][1].extend(errors)
    (vanilla, vanilla_errors), (horizon, horizon_errors) = sides['vanilla'], sides['horizon']
    (build / 'vanilla.txt').write_text('\n'.join(vanilla) + '\n', encoding='utf-8')
    (build / 'horizon.txt').write_text('\n'.join(horizon) + '\n', encoding='utf-8')
    errors = vanilla_errors + horizon_errors
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:6000])
    if sum('[Server' in l or 'chat.type.admin' in l for l in vanilla) < 5:
        raise AssertionError(f'vanilla sent the operators too little: the script did not run (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='The system chat two operators get on a dedicated server while the console and one of them run '
              'commands: broadcastToAdmins for the console (broadcast-console-to-ops on and off) and a player, '
              'queries, deop and op, send_command_feedback off and on, and save-all.'))
    print(f'server ops feedback: {len(vanilla)} chat lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
