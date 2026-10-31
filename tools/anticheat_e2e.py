#!/usr/bin/env python3
"""The dedicated server's anticheat (docs/anticheat.md), end to end: a real `horizon-client --dedicated`
and horizon-bots, legit bots beside cheating ones.

    python3 tools/anticheat_e2e.py --bin-dir target/fast            # every check
    python3 tools/anticheat_e2e.py --bin-dir target/fast --check timer

Checks:
- off:   no `[anticheat]` in server.toml: the anticheat says nothing, writes no log and kicks no one, the
         cheating bot included (vanilla's checks only).
- timer: `[anticheat]` on: a bot ticking 1.5 times as fast as vanilla's client is flagged, sent back,
         reported to the console and kicked; the legit bots (walking and exploring at 20 Hz) are never
         flagged.
- ground: `[anticheat]` on, survival (allow-flight on, so vanilla's floating kick stays out of it): a bot
         hovering 24 blocks up that claims the ground in every move (no-fall) and one standing on water
         that does (jesus) are flagged, their claim dropped, and kicked; bots walking on the ground and
         standing still are never flagged.
- prediction: `[anticheat]` on, survival: a real Horizon client, through a relay that holds every packet
         20-120 ms, walks, sprint-jumps, strafes and sneaks, with effects given and cleared, over ice and
         soul sand, swims, walks into cows, has blocks changed around it, flies in creative, glides with an
         elytra and a firework, launches with riptide and rides a boat: it is never flagged. A bot, which
         moves by its own rules and sends no keys, is flagged and sent back.
- prediction-vanilla (only with --check): the same with the real vanilla 26.3 client (tools/vanilla_client.py, from the owner's
         own files; on macOS from the logged-in screen session, even a locked one), its keys pressed by
         VanillaMover (mods/horizon/src/test/java): it walks, sprint-jumps, strafes and sneaks, with Speed
         given and cleared, over packed ice, swims, walks into cows, jumps at a block set in front of it and
         glides with an elytra and a firework, through the same relay: it is never flagged.

Each server runs in a fresh folder with fresh HORIZON_HOME and XDG folders (lab.game_env), on a free port,
in a flat world (creative unless the check says otherwise: the bots fly). Build first:
    cargo build --profile fast -p horizon-client --bin horizon-client --bin horizon-ctl
    cargo build --profile fast -p horizon-bots
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import queue
import random
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'spawn-protection': '0',
    'view-distance': '4',
    'simulation-distance': '4',
    'max-players': '100',
}
# The bots move by their own rules, not vanilla's: the prediction, which plays each move again with
# vanilla's movement code, would flag every one of them. Its own check runs real clients.
ON = '''[anticheat]
enabled = true
[anticheat.prediction]
enabled = false
'''


class Failed(Exception):
    pass


def start(bins, folder, anticheat, out, name, **extra):
    folder.mkdir(parents=True)
    port = lab.free_port()
    props = dict(PROPERTIES, **extra, **{'server-port': str(port)})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (folder / 'eula.txt').write_text('eula=true\n')
    if anticheat is not None:
        (folder / 'server.toml').write_text(anticheat)
    command = [bins / f'horizon-client{EXE}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    server = Server(command, folder, out / f'{name}-server.log')
    server.wait_for(DONE, 300)
    return server, port


def bots(bins, port, out, name, count, behaviour, seconds, *extra):
    """horizon-bots, run to the end; its report."""
    report = out / f'{name}.json'
    subprocess.run([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', str(count),
                    '--behaviour', behaviour, '--seconds', str(seconds), '--join-every', '0.2',
                    '--name', name, '--report', report, *extra],
                   check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=seconds + 180)
    return json.loads(report.read_text())


def run_pair(bins, folder, out, name, anticheat, seconds):
    """Legit bots and a timer cheat on one server, at the same time; (server lines, legit, cheat, log)."""
    server, port = start(bins, folder, anticheat, out, name)
    try:
        with ThreadPoolExecutor(2) as pool:
            legit = pool.submit(bots, bins, port, out, f'{name}Legit', 6, 'mixed', seconds)
            cheat = pool.submit(bots, bins, port, out, f'{name}Timer', 1, 'walk', seconds, '--timer', '1.5')
            legit, cheat = legit.result(), cheat.result()
    finally:
        server.stop()
    log = folder / 'logs/anticheat.log'
    return server.since(0), legit, cheat, log.read_text() if log.exists() else None


def check_off(bins, scratch, out, seconds):
    lines, legit, cheat, log = run_pair(bins, scratch / 'off', out, 'Off', None, seconds)
    if log is not None:
        raise Failed(f'off: an anticheat log was written:\n{log}')
    noise = [l for l in lines if 'Anticheat' in l]
    if noise:
        raise Failed('off: the anticheat spoke:\n' + '\n'.join(noise))
    for bot in legit['per_bot'] + cheat['per_bot']:
        if bot['error']:
            raise Failed(f'off: {bot["name"]} was dropped: {bot["error"]}')
    print(f'off: {legit["joined"] + cheat["joined"]} bots, the 1.5x one included, never flagged or kicked')


def check_timer(bins, scratch, out, seconds):
    lines, legit, cheat, log = run_pair(bins, scratch / 'timer', out, 'On', ON, seconds)
    if not any('Anticheat on' in l for l in lines):
        raise Failed('timer: the server did not say the anticheat is on')
    log = log or ''
    for bot in legit['per_bot']:
        if bot['name'] in log:
            raise Failed(f'timer: legit {bot["name"]} was flagged:\n{log}')
        if bot['error']:
            raise Failed(f'timer: legit {bot["name"]} was dropped: {bot["error"]}')
    cheater = cheat['per_bot'][0]
    flags = log.count(f'{cheater["name"]} failed timer')
    if flags < 3:
        raise Failed(f'timer: the 1.5x bot was flagged {flags} times:\n{log}')
    if 'sent back' not in log or cheater['pulled_back'] == 0:
        raise Failed('timer: the 1.5x bot was never sent back')
    if not any(f'[Anticheat] {cheater["name"]} failed timer' in l for l in lines):
        raise Failed('timer: no alert on the console')
    if 'kicked' not in log or 'disconnected' not in (cheater['error'] or ''):
        raise Failed(f'timer: the 1.5x bot was not kicked ({cheater["error"]})')
    print(f'timer: 1.5x bot flagged {flags} times, sent back {cheater["pulled_back"]} times, alerted and '
          f'kicked; {legit["joined"]} legit bots never flagged')


def check_ground(bins, scratch, out, seconds):
    folder = scratch / 'ground'
    server, port = start(bins, folder, ON, out, 'ground', **{'gamemode': 'survival', 'allow-flight': 'true'})
    try:
        with ThreadPoolExecutor(4) as pool:
            runs = {
                'Walk': pool.submit(bots, bins, port, out, 'Walk', 3, 'walk', seconds, '--height', '0', '--ground'),
                'Stand': pool.submit(bots, bins, port, out, 'Stand', 2, 'stand', seconds),
                'NoFall': pool.submit(bots, bins, port, out, 'NoFall', 1, 'idle', seconds, '--ground'),
                'Jesus': pool.submit(bots, bins, port, out, 'Jesus', 1, 'idle', seconds, '--height', '0',
                                     '--ground'),
            }
            # The jesus bot goes far from the others, then the ground under it turns to water.
            server.wait_for('Jesus0 joined the game', 120)
            server.console('tp Jesus0 200.5 -60 200.5')
            time.sleep(4)
            server.console('fill 196 -61 196 204 -61 204 minecraft:water')
            runs = {k: v.result() for k, v in runs.items()}
    finally:
        server.stop()
    log = folder / 'logs/anticheat.log'
    log = log.read_text() if log.exists() else ''
    for name in ('Walk', 'Stand'):
        for bot in runs[name]['per_bot']:
            if bot['name'] in log:
                raise Failed(f'ground: legit {bot["name"]} was flagged:\n{log}')
            if bot['error']:
                raise Failed(f'ground: legit {bot["name"]} was dropped: {bot["error"]}')
    for name, where in (('NoFall', 'in the air'), ('Jesus', 'on a fluid')):
        cheater = runs[name]['per_bot'][0]
        flags = log.count(f'{cheater["name"]} failed ground')
        if flags < 3 or f'claims the ground {where}' not in log:
            raise Failed(f'ground: {cheater["name"]} was flagged {flags} times ({where}):\n{log}')
        if 'disconnected' not in (cheater['error'] or ''):
            raise Failed(f'ground: {cheater["name"]} was not kicked ({cheater["error"]})')
    walkers = runs['Walk']['joined'] + runs['Stand']['joined']
    print(f'ground: the no-fall and jesus bots are flagged and kicked; {walkers} bots on the ground never flagged')


class Jitter:
    """A relay to the server on `port` that holds everything it carries 20-120 ms, in order: a player on
    a bad connection."""

    def __init__(self, port):
        self.server = port
        self.listener = socket.create_server(('127.0.0.1', 0))
        self.port = self.listener.getsockname()[1]
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while True:
            try:
                client, _ = self.listener.accept()
            except OSError:
                return
            upstream = socket.create_connection(('127.0.0.1', self.server))
            for a, b in ((client, upstream), (upstream, client)):
                held = queue.Queue()
                threading.Thread(target=self.read, args=(a, held), daemon=True).start()
                threading.Thread(target=self.write, args=(b, held), daemon=True).start()

    @staticmethod
    def read(conn, held):
        due = 0.0
        while True:
            try:
                data = conn.recv(65536)
            except OSError:
                data = b''
            due = max(due, time.monotonic() + random.uniform(0.02, 0.12))
            held.put((due, data))
            if not data:
                return

    @staticmethod
    def write(conn, held):
        while True:
            due, data = held.get()
            time.sleep(max(0.0, due - time.monotonic()))
            if not data:
                conn.close()
                return
            try:
                conn.sendall(data)
            except OSError:
                return

    def close(self):
        self.listener.close()


def honest_play(game, console):
    """What the honest client does on the prediction's server, in a survival, peaceful flat world."""
    def at(command):
        console(f'execute as Walker at Walker run {command}')
    game.raw('move', 'forward', 2)
    game.raw('move', 'forward', 3, '--sprint', '--jump')
    game.raw('turn', 90, 0, '--over', 1)
    game.raw('move', 'left', 2)
    game.raw('move', 'back', 2, '--sneak')
    console('effect give Walker minecraft:speed 30 2')
    console('effect give Walker minecraft:jump_boost 30 1')
    game.wait(10)
    game.raw('move', 'forward', 2, '--sprint', '--jump')
    console('effect clear Walker')
    game.wait(10)
    # Ice, then soul sand, under a sprint; a pool to swim across.
    at('fill ~-3 ~-1 ~-3 ~3 ~-1 ~12 minecraft:packed_ice')
    game.wait(10)
    game.raw('move', 'forward', 2, '--sprint', '--jump')
    at('fill ~-3 ~-1 ~-3 ~3 ~-1 ~12 minecraft:soul_sand')
    game.wait(10)
    game.raw('move', 'forward', 2, '--sprint')
    at('fill ~-3 ~-3 ~-3 ~3 ~-1 ~12 minecraft:water')
    game.wait(20)
    game.raw('move', 'forward', 3, '--jump')
    # Riptide, from the pool: a trident drawn for 25 ticks.
    console('item replace entity Walker weapon.mainhand with minecraft:trident'
            '[minecraft:enchantments={"minecraft:riptide":3}]')
    game.wait(5)
    game.script(['mouse right down', 'wait 25', 'mouse right up', 'wait 60'])
    console('item replace entity Walker weapon.mainhand with minecraft:air')
    at('fill ~-4 ~-4 ~-4 ~4 ~-1 ~13 minecraft:stone')
    game.wait(20)
    # Cows to walk into, blocks changed in front, under and around.
    for ahead in (1, 2):
        at(f'summon minecraft:cow ^ ^ ^{ahead}')
    game.wait(10)
    game.raw('move', 'forward', 2)
    console('kill @e[type=minecraft:cow]')
    at('setblock ^ ^ ^1 minecraft:stone')
    game.wait(2)
    game.raw('move', 'forward', 1, '--jump')
    at('fill ~-1 ~-1 ~-1 ~1 ~-1 ~1 minecraft:air')
    game.wait(30)
    at('fill ~-2 ~-1 ~-2 ~2 ~-1 ~2 minecraft:stone')
    game.wait(10)
    # Creative flight (double jump), then back to survival on the ground.
    console('gamemode creative Walker')
    game.wait(10)
    game.script(['key space tap', 'wait 2', 'key space tap', 'wait 2', 'key space down', 'wait 10',
                 'key space up', 'key w down', 'wait 30', 'key w up', 'key shift down', 'wait 40',
                 'key shift up'])
    console('gamemode survival Walker')
    game.wait(60)
    # An elytra from 60 blocks up (Resistance V against the landing), with a firework, then a teleport
    # back down.
    console('effect give Walker minecraft:resistance 60 4 true')
    console('item replace entity Walker armor.chest with minecraft:elytra')
    console('item replace entity Walker weapon.mainhand with minecraft:firework_rocket')
    at('tp @s ~ ~60 ~ 0 0')
    game.wait(6)
    game.script(['key space down', 'wait 1', 'key space up', 'wait 20', 'mouse right click', 'wait 40'])
    at('tp @s ~ -60 ~')
    game.wait(40)
    console('item replace entity Walker armor.chest with minecraft:air')
    console('effect clear Walker')
    # A boat on a pool: in, paddle, out.
    from lan_e2e import board, leave_vehicle  # noqa: E402
    x, y, z = game('state')['player']['position']
    at('fill ~-2 ~-1 ~-2 ~2 ~-1 ~10 minecraft:water')
    console(f'summon minecraft:oak_boat {x:.2f} {y - 0.1:.2f} {z + 2:.2f}')
    game.wait(30)
    board(game, 'oak_boat', (x, y, z + 2))
    game.script(['key w down', 'wait 40', 'key w up'])
    leave_vehicle(game, 'boat')
    game.wait(20)
    at('fill ~-3 ~-2 ~-3 ~3 ~-1 ~11 minecraft:stone')
    game.raw('move', 'back', 2, '--sprint')
    game.wait(40)


def check_prediction(bins, scratch, out, seconds):
    folder = scratch / 'prediction'
    # The other checks stay out of it: this one is about the prediction alone.
    toml = '[anticheat]\nenabled = true\n[anticheat.timer]\nenabled = false\n[anticheat.ground]\nenabled = false\n'
    server, port = start(bins, folder, toml, out, 'prediction', gamemode='survival', difficulty='peaceful')
    from lan_e2e import Game  # noqa: E402 (only this check drives a client)
    game = Game('client', bins / f'horizon-ctl{EXE}', lab.RESOURCES.resolve())
    for key in ('XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'HORIZON_LAUNCHER_HOME'):
        game.d.env[key] = str(game.work / key.lower())
    game.d.env['HORIZON_LAUNCHER_NO_SELF_UPDATE'] = '1'
    relay = Jitter(port)
    try:
        try:
            game('launch', wait=False, offscreen=sys.platform == 'darwin')
            game.raw('connect', f'127.0.0.1:{relay.port}', 'Walker')
            game.raw('wait-for', 'loaded', '--timeout', 300)
            game.wait(60)
            honest_play(game, server.console)
        except Exception as e:
            raise Failed(f'prediction: the honest client could not play: {e}') from e
        finally:
            (out / 'prediction-client.log').write_text(game.close())
            relay.close()
            shutil.rmtree(game.work, ignore_errors=True)
            log = folder / 'logs/anticheat.log'
            if log.exists():
                shutil.copy(log, out / 'prediction-anticheat.log')
        if log.exists() and 'Walker' in log.read_text():
            raise Failed(f'prediction: the honest client was flagged:\n{log.read_text()}')
        cheat = bots(bins, port, out, 'Cheat', 1, 'walk', seconds)
    finally:
        server.stop()
    log = log.read_text() if log.exists() else ''
    cheater = cheat['per_bot'][0]
    flags = log.count(f'{cheater["name"]} failed prediction')
    if flags < 3 or 'sent back' not in log or cheater['pulled_back'] == 0:
        raise Failed(f'prediction: the bot was flagged {flags} times, sent back {cheater["pulled_back"]}:\n{log}')
    print(f'prediction: the honest client (20-120 ms jitter) was never flagged; the bot was flagged {flags} '
          f'times and sent back {cheater["pulled_back"]} times')


class VanillaClient:
    """The vanilla client, joined to `address` as `name`, its keys pressed by VanillaMover from `send`."""

    def __init__(self, address, name, work, log):
        import vanilla_client  # noqa: E402 (only this check runs it)
        classes = work / 'classes'
        classes.mkdir(parents=True)
        jars = os.pathsep.join(map(str, vanilla_client.classpath()))
        release = str(lab.config()[0]['minecraft']['java_major'])
        mover = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMover.java'
        subprocess.run(['javac', '--release', release, '-proc:none', '-nowarn', '-cp', jars, '-d', classes, mover],
                       check=True, capture_output=True)
        args = argparse.Namespace(name=name, join=address, world=None, game_dir=work / 'client', assets=None,
                                  heap='2G', width=854, height=480, lan_port=None, backend=None)
        args.game_dir.mkdir()
        vanilla_client.write_options(args.game_dir, {})
        command = vanilla_client.command(args)
        cp = command.index('-cp') + 1
        command[cp] = os.pathsep.join([str(classes), command[cp]])
        command[command.index('net.minecraft.client.main.Main')] = 'dev.horizonlab.parity.VanillaMover'
        self.log = open(log, 'w')
        self.ready = threading.Event()
        self.positions = []
        self.process = subprocess.Popen(command, cwd=args.game_dir, env=vanilla_client.environment(), text=True,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        for line in self.process.stdout:
            self.log.write(line)
            if '[VanillaMover] ready' in line:
                self.ready.set()
            elif '[VanillaMover] at ' in line:
                self.positions.append([float(v) for v in line.split('[VanillaMover] at ')[1].split()[:3]])

    def __call__(self, *lines, then=0.0):
        """Send `lines` ("down forward", "look 90 0"...), then wait `then` seconds."""
        for line in lines:
            self.process.stdin.write(line + '\n')
        self.process.stdin.flush()
        time.sleep(then)

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(20)
        except subprocess.TimeoutExpired:
            self.process.kill()
        self.log.close()


def vanilla_play(keys, console):
    """What the vanilla client does: a second or a few per step, as a player would."""
    at = lambda command: console(f'execute as Vanilla at Vanilla run {command}')  # noqa: E731
    keys('look 0 0', 'down forward', then=3)
    keys('down sprint', 'down jump', then=4)
    keys('up jump', 'up sprint', 'down left', 'look 45 0', then=2)
    keys('up left', 'up forward', 'down sneak', 'down back', then=2)
    keys('up sneak', 'up back')
    console('effect give Vanilla minecraft:speed 30 2 true')
    keys('down forward', 'down jump', then=4)
    keys('up jump', 'up forward')
    console('effect clear Vanilla')
    at('fill ~-3 ~-1 ~-3 ~3 ~-1 ~12 minecraft:packed_ice')
    keys('down forward', 'down sprint', 'down jump', then=3)
    keys('up jump', 'up sprint', 'up forward', then=1)
    at('fill ~-3 ~-2 ~-3 ~3 ~-1 ~12 minecraft:water')
    keys('down forward', then=4)
    keys('down jump', then=2)
    keys('up jump', 'up forward')
    at('fill ~-4 ~-4 ~-4 ~4 ~-1 ~13 minecraft:stone')
    time.sleep(2)
    at('summon minecraft:cow ^ ^ ^2')
    keys('down forward', then=2)
    keys('up forward')
    console('kill @e[type=minecraft:cow]')
    at('setblock ^ ^ ^1 minecraft:stone')
    keys('down forward', 'down jump', then=2)
    keys('up forward', 'up jump', then=1)
    # An elytra from 60 blocks up (Resistance V against the landing): a jump to open it, a firework.
    console('effect give Vanilla minecraft:resistance 60 4 true')
    console('item replace entity Vanilla armor.chest with minecraft:elytra')
    console('item replace entity Vanilla weapon.mainhand with minecraft:firework_rocket')
    at('tp @s ~ ~60 ~ 0 0')
    time.sleep(1)
    keys('down jump', then=0.2)
    keys('up jump', then=0.5)
    keys('down jump', then=0.2)
    keys('up jump', then=0.5)
    keys('down use', then=0.2)
    keys('up use', then=4)
    at('tp @s ~ -60 ~')
    time.sleep(3)


def check_prediction_vanilla(bins, scratch, out, seconds):
    folder = scratch / 'prediction-vanilla'
    toml = '[anticheat]\nenabled = true\n[anticheat.timer]\nenabled = false\n[anticheat.ground]\nenabled = false\n'
    server, port = start(bins, folder, toml, out, 'prediction-vanilla', gamemode='survival', difficulty='peaceful')
    relay = Jitter(port)
    client = None
    try:
        try:
            client = VanillaClient(f'127.0.0.1:{relay.port}', 'Vanilla', folder / 'vanilla',
                                   out / 'prediction-vanilla-client.log')
            if not client.ready.wait(300):
                raise Failed('prediction-vanilla: the vanilla client never got into the world')
            time.sleep(5)
            client('where')
            vanilla_play(client, server.console)
            client('where', then=1)
        finally:
            if client:
                client.close()
            relay.close()
    finally:
        server.stop()
    log = folder / 'logs/anticheat.log'
    if log.exists():
        shutil.copy(log, out / 'prediction-vanilla-anticheat.log')
        if 'Vanilla' in log.read_text():
            raise Failed(f'prediction-vanilla: the vanilla client was flagged:\n{log.read_text()}')
    if len(client.positions) < 2:
        raise Failed('prediction-vanilla: the vanilla client did not say where it was')
    (x0, _, z0), (x1, _, z1) = client.positions[0], client.positions[-1]
    walked = ((x1 - x0) ** 2 + (z1 - z0) ** 2) ** 0.5
    if walked < 20:
        raise Failed(f'prediction-vanilla: the vanilla client only got {walked:.1f} blocks away: its keys did nothing')
    print(f'prediction-vanilla: the vanilla client (20-120 ms jitter) was never flagged; it ended {walked:.0f} blocks '
          'from where it joined')


CHECKS = {'off': check_off, 'timer': check_timer, 'ground': check_ground, 'prediction': check_prediction,
          'prediction-vanilla': check_prediction_vanilla}
# Run when no --check is given: the vanilla client needs a screen session, so it runs only when asked.
DEFAULT = ['ground', 'off', 'prediction', 'timer']


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--check', action='append', choices=sorted(CHECKS))
    parser.add_argument('--seconds', type=int, default=40)
    parser.add_argument('--out', default=str(lab.LOCAL / 'anticheat-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    # One set of scratch HORIZON_HOME and XDG folders for every server below.
    os.environ.update(lab.game_env())
    failed = []
    with tempfile.TemporaryDirectory(prefix='anticheat-e2e-') as scratch:
        for name in args.check or DEFAULT:
            try:
                CHECKS[name](bins, Path(scratch), out, args.seconds)
            except (Failed, RuntimeError) as e:
                failed.append(name)
                print(f'FAILED {e}')
    if failed:
        sys.exit(f'anticheat_e2e: {", ".join(failed)} failed (logs in {out})')


if __name__ == '__main__':
    main()
