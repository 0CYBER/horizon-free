#!/usr/bin/env python3
"""Server resource packs end to end: a real Horizon client joins a real dedicated server (vanilla 26.3's, or
Horizon's with --horizon-server BIN) that offers a test pack from a local HTTP server, through tools/net_capture.py,
which records the statuses the client sends back.

The pack is built here: dirt recoloured blue, and three lang strings (dirt's name, the pause menu's "Back to Game",
the server list's "Join Server"). Each scenario sets the server list entry's Server Resource Packs setting in
servers.dat, joins, answers the prompt the way a player would (horizon-ctl clicks its buttons), and checks the
statuses against what vanilla's client sends (ServerPackManager):

- required, Prompt, Disconnect: no status, the disconnected screen says "This server requires a custom resource
  pack", the setting stays Prompt;
- required, Prompt, Proceed: ACCEPTED, DOWNLOADED, SUCCESSFULLY_LOADED; the setting becomes Enabled, the file is in
  downloads/ under its SHA-1 with a log.json entry, the HTTP request carries vanilla's X-Minecraft-* headers, the
  blue dirt is on screen (screenshot) and the pause menu says the pack's "Back to Game"; after leaving, the server
  list says "Join Server" again (the pack is gone);
- required, Enabled: no prompt, the same three statuses, and no second download (the cache);
- required with a wrong SHA-1: ACCEPTED, FAILED_DOWNLOAD, and the player still joins;
- optional, Prompt, No: DECLINED, joins, the setting becomes Disabled; then joining again sends DECLINED without
  asking; then Prompt and Yes: the pack (no hash given) is downloaded and loaded;
- optional on a dead URL: ACCEPTED, FAILED_DOWNLOAD, joins.

  python3 tools/server_pack_e2e.py                                   # vanilla's server
  python3 tools/server_pack_e2e.py --horizon-server target/release/horizon-client

Every game and server here runs in fresh temporary folders (HORIZON_HOME, HORIZON_LAUNCHER_HOME, XDG_*), with no
audio and no self-update. Needs .local (the vanilla jars and prepared resources) and a built horizon-client and
horizon-ctl (--bin DIR, default target/fast).
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import net_capture  # noqa: E402
import test_rust_parity as base  # noqa: E402
from test_dedicated_server_parity import Server, free_port  # noqa: E402

# ServerboundResourcePackPacket.Action, by ordinal.
ACTIONS = ['successfully_loaded', 'declined', 'failed_download', 'accepted', 'downloaded', 'invalid_url',
           'failed_reload', 'discarded']
LOADED = ['accepted', 'downloaded', 'successfully_loaded']
FAILED = ['accepted', 'failed_download']
REQUIRED_REASON = 'This server requires a custom resource pack'
PLAYER = 'Steve'
PROPERTIES = {
    'online-mode': 'false', 'white-list': 'false', 'level-seed': '12345', 'level-type': 'minecraft\\:flat',
    'generate-structures': 'false', 'view-distance': '3', 'simulation-distance': '3', 'spawn-protection': '0',
    'pause-when-empty-seconds': '-1', 'gamemode': 'creative',
}


class Failed(Exception):
    pass


def check(condition, what):
    if not condition:
        raise Failed(what)
    print(f'  ok: {what}')


# The pack.

def png(width, height, pixel):
    rows = b''.join(b'\0' + b''.join(bytes(pixel(x, y)) for x in range(width)) for y in range(height))

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    header = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')


def build_pack(path):
    """The test pack, byte for byte the same every time (fixed dates), and its SHA-1."""
    def blue(x, y):
        return (40, 90, 230, 255) if (x // 4 + y // 4) % 2 else (20, 50, 170, 255)
    files = {
        'pack.mcmeta': json.dumps({'pack': {'description': 'Horizon server pack test',
                                            'min_format': 97, 'max_format': 97}}).encode(),
        'assets/minecraft/textures/block/dirt.png': png(16, 16, blue),
        'assets/minecraft/lang/en_us.json': json.dumps({
            'block.minecraft.dirt': 'Blue Test Dirt',
            'menu.returnToGame': 'Back to Game (server pack)',
            'selectServer.select': 'Join Server (server pack)',
        }).encode(),
    }
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        for name, data in files.items():
            z.writestr(zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0)), data)
    return hashlib.sha1(path.read_bytes()).hexdigest()


def decode_png(data):
    """RGB(A) pixels of an 8-bit PNG: (width, height, rows of (r, g, b))."""
    pos, idat, width = 8, b'', 0
    while pos < len(data):
        length, kind = struct.unpack('>I4s', data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        if kind == b'IHDR':
            width, height, depth, colour = struct.unpack('>IIBB', body[:10])
            assert depth == 8 and colour in (2, 6), (depth, colour)
            channels = 4 if colour == 6 else 3
        elif kind == b'IDAT':
            idat += body
        pos += 12 + length
    raw, stride, rows, previous = zlib.decompress(idat), width * channels, [], bytearray(width * channels)
    for y in range(height):
        kind, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up, corner = previous[i], previous[i - channels] if i >= channels else 0
            if kind == 1:
                line[i] = (line[i] + left) & 255
            elif kind == 2:
                line[i] = (line[i] + up) & 255
            elif kind == 3:
                line[i] = (line[i] + (left + up) // 2) & 255
            elif kind == 4:
                p = left + up - corner
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - corner)
                line[i] = (line[i] + (left if pa <= pb and pa <= pc else up if pb <= pc else corner)) & 255
        rows.append([tuple(line[x * channels:x * channels + 3]) for x in range(width)])
        previous = line
    return width, height, rows


def blue_share(path):
    """How much of the picture's lower middle (the ground under a player looking down) is blue."""
    width, height, rows = decode_png(path.read_bytes())
    samples = [rows[y][x] for y in range(height // 2 + height // 8, height - height // 8, 4)
               for x in range(width // 4, 3 * width // 4, 4)]
    return sum(b > r + 60 and b > g + 30 for r, g, b in samples) / len(samples)


# The HTTP server the pack comes from.

class Files(http.server.ThreadingHTTPServer):
    def __init__(self, files):
        self.files, self.requests = files, []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(handler):
                headers = {k: v for k, v in handler.headers.items() if k.lower().startswith(('x-minecraft', 'user-agent'))}
                self.requests.append((handler.path, headers))
                data = self.files.get(handler.path)
                if data is None:
                    handler.send_error(404)
                    return
                handler.send_response(200)
                handler.send_header('Content-Type', 'application/zip')
                handler.send_header('Content-Length', str(len(data)))
                handler.end_headers()
                handler.wfile.write(data)

            def log_message(handler, *args):
                pass
        super().__init__(('127.0.0.1', 0), Handler)
        threading.Thread(target=self.serve_forever, daemon=True).start()

    def url(self, path):
        return f'http://127.0.0.1:{self.server_address[1]}{path}'


# servers.dat (NbtIo.write: uncompressed, a named root compound).

def nbt_string(s):
    data = s.encode()
    return struct.pack('>H', len(data)) + data


def write_servers(path, ip, setting):
    """One entry for ip; setting True (Enabled), False (Disabled) or None (Prompt: no acceptTextures)."""
    entry = b'\x08' + nbt_string('name') + nbt_string('Pack test') + b'\x08' + nbt_string('ip') + nbt_string(ip)
    if setting is not None:
        entry += b'\x01' + nbt_string('acceptTextures') + bytes([int(setting)])
    root = b'\x09' + nbt_string('servers') + b'\x0a' + struct.pack('>i', 1) + entry + b'\x00'
    path.write_bytes(b'\x0a' + nbt_string('') + root + b'\x00')


def read_setting(path):
    data = path.read_bytes()
    key = b'\x01' + nbt_string('acceptTextures')
    at = data.find(key)
    return None if at < 0 else bool(data[at + len(key)])


# The game.

class Game:
    def __init__(self, bins, resources, work):
        self.home = work / 'home'
        for d in ('home', 'launcher', 'data', 'config'):
            (work / d).mkdir(parents=True)
        self.env = lab.game_env(HORIZON_HOME=self.home, HORIZON_LAUNCHER_HOME=work / 'launcher',
                                XDG_DATA_HOME=work / 'data', XDG_CONFIG_HOME=work / 'config', HORIZON_TELEMETRY_URL='off')
        self.base = [str(bins / 'horizon-ctl'), '--socket', str(work / 'c.sock')]
        self.log = work / 'c.log'
        reply = self('launch', '--game', bins / 'horizon-client', '--offscreen', '--resources', resources)
        self.pid = reply.get('pid')

    def __call__(self, *words, ok=True, timeout=300):
        result = subprocess.run(self.base + [str(w) for w in words], cwd=self.home, env=self.env,
                                capture_output=True, text=True, timeout=timeout)
        lines = result.stdout.strip().splitlines()
        reply = json.loads(lines[-1]) if lines else {'error': result.stderr.strip()}
        print(f'    {" ".join(map(str, words))[:80]} -> {json.dumps(reply)[:200]}')
        if ok and (result.returncode or 'error' in reply):
            raise Failed(f'{" ".join(map(str, words))}: {reply}')
        return reply

    def labels(self):
        return [w.get('label') for w in self('ui').get('widgets', [])]

    def wait_widget(self, widget, timeout=60):
        return self('wait-for', 'widget', widget, '--timeout', timeout, ok=False)

    def quit(self):
        try:
            self('quit', ok=False, timeout=60)
        except subprocess.TimeoutExpired:
            pass
        if self.pid:
            for _ in range(40):
                try:
                    os.kill(self.pid, 0)
                except OSError:
                    return
                time.sleep(0.5)
            os.kill(self.pid, 9)


def statuses(capture, conn):
    """The resource pack statuses the client sent on one connection: (id, action), in order."""
    out = []
    for e in net_capture.load(capture):
        if e.get('conn') == conn and e.get('dir') == 'c2s' and e.get('name') == 'resource_pack':
            data = bytes.fromhex(e['data'])
            out.append((data[:16].hex(), ACTIONS[data[16]]))
    return out


def connections(capture):
    return max([e['conn'] for e in net_capture.load(capture) if 'conn' in e], default=0)


class Run:
    def __init__(self, args, out):
        self.args, self.out = args, out
        lock, jars, libraries = base.verify_artifacts()
        deps = os.pathsep.join(map(str, [jars['server'], *libraries]))
        self.vanilla = ['java', '-Xmx2g', '-cp', deps, 'net.minecraft.server.Main', 'nogui']
        self.resources = lab.LOCAL / 'horizon-client' / lab.VERSION
        self.files = {}
        self.http = Files(self.files)
        self.sha1 = build_pack(out / 'test-pack.zip')
        self.files['/test-pack.zip'] = (out / 'test-pack.zip').read_bytes()
        self.files['/optional-pack.zip'] = self.files['/test-pack.zip']
        dead = free_port()  # nothing listens there
        self.packs = {
            'required': {'resource-pack': self.http.url('/test-pack.zip'), 'resource-pack-sha1': self.sha1,
                         'require-resource-pack': 'true', 'resource-pack-id': '5b2c7a4e-1f3d-4c8b-9a6e-2d4f8b1c3e5a',
                         'resource-pack-prompt': '["Blue dirt for ",{"text":"this test","color":"aqua"}]'},
            'bad-sha1': {'resource-pack': self.http.url('/test-pack.zip'), 'resource-pack-sha1': '0' * 40,
                         'require-resource-pack': 'true'},
            'optional': {'resource-pack': self.http.url('/optional-pack.zip'), 'require-resource-pack': 'false'},
            'dead-url': {'resource-pack': f'http://127.0.0.1:{dead}/gone.zip', 'require-resource-pack': 'false'},
        }
        self.results = []

    def start_server(self, kind, work):
        directory = work / f'server-{kind}'
        directory.mkdir()
        port = free_port()
        props = dict(PROPERTIES, **self.packs[kind], **{'server-port': str(port)})
        (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                     encoding='iso-8859-1')
        (directory / 'eula.txt').write_text('eula=true\n')
        if self.args.horizon_server:
            command = [self.args.horizon_server, '--resources', self.resources, '--dedicated', '--nogui']
        else:
            command = self.vanilla
        server = Server(command, directory, f'server-{kind}')
        server.wait_for(r'Done \(', timeout=300)
        return server, port

    def join(self, game, address, scenario):
        """Connects; returns the capture's connection number."""
        game('connect', address, PLAYER)
        deadline = time.time() + 30
        while connections(self.capture) <= self.conn and time.time() < deadline:
            time.sleep(0.2)
        self.conn = connections(self.capture)
        print(f'  [{scenario}] connection {self.conn}')
        return self.conn

    def scenario(self, name, fn):
        print(f'- {name}')
        try:
            fn()
            self.results.append((name, 'pass', ''))
        except Exception as e:  # noqa: BLE001: report every scenario
            print(f'  FAILED: {e}')
            self.results.append((name, 'FAIL', str(e)[:300]))
            if self.game('state', ok=False).get('screen') not in (None, 'title'):
                self.leave(self.game)

    def wait_statuses(self, conn, want, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            got = [a for _, a in statuses(self.capture, conn)]
            if len(got) >= len(want) and got[:len(want)] == want or got[:1] and got != want[:len(got)]:
                break
            time.sleep(0.3)
        time.sleep(1.5)  # nothing more comes after the last one
        got = [a for _, a in statuses(self.capture, conn)]
        check(got == want, f'statuses {got} == {want}')

    def leave(self, game):
        game('disconnect', ok=False)
        time.sleep(1)

    def run(self):
        work = Path(tempfile.mkdtemp(prefix='hz-server-packs-'))
        print(f'scratch: {work}')
        # The servers too get scratch homes (the game sets its own below).
        for var, sub in (('HORIZON_HOME', 'home'), ('HORIZON_LAUNCHER_HOME', 'launcher'), ('XDG_DATA_HOME', 'data'),
                         ('XDG_CONFIG_HOME', 'config')):
            (work / 'servers' / sub).mkdir(parents=True)
            os.environ[var] = str(work / 'servers' / sub)
        os.environ.update(lab.game_env(HORIZON_TELEMETRY_URL='off'))
        game = self.game = Game(Path(self.args.bin), self.resources, work / 'game')
        servers_dat = game.home / 'servers.dat'
        downloads = game.home / 'downloads'
        try:
            for kind, scenarios in self.plan(game, servers_dat, downloads):
                server, port = self.start_server(kind, work)
                # One capture per server: its connections are numbered from 1.
                self.capture, self.conn = self.out / f'capture-{kind}.jsonl', 0
                self.capture.touch()
                listen = free_port()
                stop = net_capture.start_background(listen, f'127.0.0.1:{port}', self.capture)
                self.address = f'127.0.0.1:{listen}'
                try:
                    self.server = server
                    for name, fn in scenarios:
                        self.scenario(f'{kind}: {name}', fn)
                finally:
                    stop.set()
                    server.stop()
                    shutil.copy(server.log_path, self.out / server.log_path.name)
        finally:
            game.quit()
            if game.log.exists():
                shutil.copy(game.log, self.out / 'client.log')
            if downloads.exists():
                shutil.copytree(downloads, self.out / 'downloads')
        return work

    def plan(self, game, servers_dat, downloads):
        sha1 = self.sha1

        def required_disconnect():
            write_servers(servers_dat, self.address, None)
            conn = self.join(game, self.address, 'decline')
            check(game.wait_widget('disconnect').get('ok'), 'the required prompt opened')
            labels = game.labels()
            check('Proceed' in labels and 'Disconnect' in labels, f'Proceed and Disconnect buttons: {labels}')
            game('screenshot', self.out / 'prompt-required.png')
            game('click', 'disconnect')
            self.wait_statuses(conn, [])
            screen = json.dumps(game('screen'))
            check(REQUIRED_REASON in screen, 'the disconnected screen says "This server requires a custom resource pack"')
            check(read_setting(servers_dat) is None, 'the setting stays Prompt')
            check(not any(p == '/test-pack.zip' for p, _ in self.http.requests), 'nothing was downloaded')

        def required_proceed():
            conn = self.join(game, self.address, 'proceed')
            check(game.wait_widget('proceed').get('ok'), 'the required prompt opened')
            game('click', 'proceed')
            game('wait-for', 'loaded', '--timeout', 180)
            self.wait_statuses(conn, LOADED)
            check(read_setting(servers_dat) is True, 'the setting became Enabled (acceptTextures 1)')
            pack_id = self.packs['required']['resource-pack-id']
            check((downloads / pack_id / sha1).exists(), f'downloads/{pack_id}/{sha1} is cached')
            log = json.loads((downloads / 'log.json').read_text().splitlines()[-1])
            check(log.get('id') == pack_id and log.get('hash') == sha1
                  and log.get('file', {}).get('name') == f'{pack_id}/{sha1}', f'log.json entry {log}')
            path, headers = [r for r in self.http.requests if r[0] == '/test-pack.zip'][-1]
            want = {'X-Minecraft-Username': PLAYER, 'X-Minecraft-Version': lab.VERSION,
                    'X-Minecraft-Version-ID': lab.VERSION, 'X-Minecraft-Pack-Format': '97.1',
                    'User-Agent': f'Minecraft Java/{lab.VERSION}'}
            check(all(headers.get(k) == v for k, v in want.items()) and len(headers.get('X-Minecraft-UUID', '')) == 32,
                  f'download headers {headers}')
            self.server.console(f'execute at {PLAYER} run fill ~-6 ~-1 ~-6 ~6 ~-1 ~6 minecraft:dirt')
            game('look', 0, 90)
            game('wait', 40)
            shot = self.out / 'applied-blue-dirt.png'
            game('screenshot', shot)
            share = blue_share(shot)
            check(share > 0.5, f'the ground is the pack\'s blue dirt ({share:.0%} of the samples)')
            game('key', 'esc', 'tap')
            game('wait', 5)
            labels = game.labels()
            check('Back to Game (server pack)' in labels, f'the pause menu uses the pack\'s lang: {labels}')
            game('key', 'esc', 'tap')
            self.leave(game)
            labels = game.labels()
            check('Join Server' in labels and 'Join Server (server pack)' not in labels,
                  f'after leaving, the pack is gone: {labels}')

        def required_enabled():
            before = len(self.http.requests)
            conn = self.join(game, self.address, 'enabled')
            game('wait-for', 'loaded', '--timeout', 180)
            self.wait_statuses(conn, LOADED)
            check(len(self.http.requests) == before, 'no second download: the cached file is used')
            game('key', 'esc', 'tap')
            game('wait', 5)
            check('Back to Game (server pack)' in game.labels(), 'the pack is applied without a prompt')
            game('key', 'esc', 'tap')
            self.leave(game)

        def bad_sha1():
            write_servers(servers_dat, self.address, True)
            conn = self.join(game, self.address, 'bad-sha1')
            game('wait-for', 'loaded', '--timeout', 180)
            self.wait_statuses(conn, FAILED)
            entries = [json.loads(line) for line in (downloads / 'log.json').read_text().splitlines()]
            failed = [e for e in entries if e.get('hash') == '0' * 40]
            check(failed and failed[-1].get('error') == 'download_failed' and 'file' not in failed[-1],
                  f'log.json records the failure: {failed[-1:]}')
            self.leave(game)

        def optional_no():
            write_servers(servers_dat, self.address, None)
            conn = self.join(game, self.address, 'optional-no')
            check(game.wait_widget('no').get('ok'), 'the optional prompt opened')
            labels = game.labels()
            check('Yes' in labels and 'No' in labels, f'Yes and No buttons: {labels}')
            game('screenshot', self.out / 'prompt-optional.png')
            game('click', 'no')
            game('wait-for', 'loaded', '--timeout', 180)
            self.wait_statuses(conn, ['declined'])
            check(read_setting(servers_dat) is False, 'the setting became Disabled (acceptTextures 0)')
            self.leave(game)

        def optional_disabled():
            conn = self.join(game, self.address, 'optional-disabled')
            game('wait-for', 'loaded', '--timeout', 180)
            self.wait_statuses(conn, ['declined'])
            self.leave(game)

        def optional_yes():
            write_servers(servers_dat, self.address, None)
            conn = self.join(game, self.address, 'optional-yes')
            check(game.wait_widget('yes').get('ok'), 'the optional prompt opened')
            game('click', 'yes')
            game('wait-for', 'loaded', '--timeout', 180)
            self.wait_statuses(conn, LOADED)
            check(any(r[0] == '/optional-pack.zip' for r in self.http.requests), 'downloaded without a hash')
            self.leave(game)

        def dead_url():
            write_servers(servers_dat, self.address, True)
            conn = self.join(game, self.address, 'dead-url')
            game('wait-for', 'loaded', '--timeout', 180)
            self.wait_statuses(conn, FAILED)
            self.leave(game)

        return [
            ('required', [('Disconnect at the prompt', required_disconnect), ('Proceed', required_proceed),
                          ('Enabled, cached', required_enabled)]),
            ('bad-sha1', [('wrong SHA-1', bad_sha1)]),
            ('optional', [('No at the prompt', optional_no), ('Disabled', optional_disabled),
                          ('Yes at the prompt, no hash', optional_yes)]),
            ('dead-url', [('dead URL', dead_url)]),
        ]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', default=str(lab.cargo_target() / 'fast'), help='horizon-client and horizon-ctl')
    parser.add_argument('--horizon-server', help='a horizon-client binary to run as the dedicated server')
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-pack-e2e'))
    args = parser.parse_args()
    out = Path(args.out) / ('horizon' if args.horizon_server else 'vanilla')
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    run = Run(args, out)
    work = run.run()
    print(f'\nserver: {"horizon" if args.horizon_server else "vanilla 26.3"}; evidence in {out}')
    for name, result, detail in run.results:
        print(f'  {result:4}  {name}' + (f': {detail}' if detail else ''))
    shutil.rmtree(work, ignore_errors=True)
    if any(r != 'pass' for _, r, _ in run.results):
        sys.exit(1)


if __name__ == '__main__':
    main()
