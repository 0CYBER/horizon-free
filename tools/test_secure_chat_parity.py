#!/usr/bin/env python3
"""Secure chat on vanilla's dedicated server and Horizon's, with no real account.

A mock of the Minecraft services stands in for Mojang's: its key set (the
`authentication` / `getPublicKeys` endpoint) holds a key pair the harness
makes, so the profile keys it signs are as good as Mojang's to both servers.
Vanilla finds the mock through authlib's `-Dminecraft.api.discovery.host`,
Horizon through `HORIZON_API_DISCOVERY`. SecureChatClient (vanilla's own
codecs and chat classes) plays several players at once: each signs its chat
with a profile key the mock signed, acknowledges what it saw, and checks the
signature of every chat it receives against the sender's session from the
player list, as vanilla's client does.

Two servers, each run on vanilla then on Horizon:
- offline mode (enforce-secure-profile is then off): signed chat with the
  last-seen window and the signature cache, unsigned chat beside it, /say,
  /me and /msg with signed and unsigned arguments, a signed argument under
  the wrong name, a forged signature and the broken chain after it, and an
  acknowledgement past the window;
- online mode with enforce-secure-profile: the status and the login packet
  say so, unsigned chat and unsigned commands with a message are refused, and
  a key signed for another player is refused.

Compared: every player's transcript, the status' `enforcesSecureChat`, and
the servers' log lines about chat.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import sys
import time
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
    from .test_dedicated_server_parity import Server
except ImportError:
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from test_dedicated_server_parity import Server

NAME = 'secure-chat-parity'
SCENARIOS = {
    'offline': {'online-mode': 'false'},
    'online': {'online-mode': 'true', 'enforce-secure-profile': 'true'},
}
PROPERTIES = {
    'white-list': 'false',
    'level-seed': '12345',
    'view-distance': '3',
    'simulation-distance': '3',
    'spawn-protection': '0',
    'max-players': '8',
    'pause-when-empty-seconds': '-1',
}


def account(name):
    """The mock services' account id for a name."""
    return hashlib.md5(f'account:{name}'.encode()).hexdigest()


class Services:
    """Discovery, the key set, the session service's join and hasJoined, profiles by name."""

    def __init__(self, key_set):
        self.joined = {}
        self.lock = threading.Lock()
        services = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, status, body=None):
                data = b'' if body is None else json.dumps(body).encode()
                self.send_response(status)
                if body is not None:
                    self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                url = urllib.parse.urlparse(self.path)
                query = dict(urllib.parse.parse_qsl(url.query))
                if url.path == '/minecraft/client':
                    return self.reply(200, services.discovery())
                if url.path == '/publickeys':
                    return self.reply(200, key_set)
                if url.path.startswith('/users/profiles/minecraft/'):
                    name = urllib.parse.unquote(url.path.rsplit('/', 1)[1])
                    return self.reply(200, {'id': account(name), 'name': name})
                if url.path == '/session/minecraft/hasJoined':
                    name = query.get('username')
                    with services.lock:
                        ok = services.joined.get(name) == query.get('serverId')
                    if ok:
                        return self.reply(200, {'id': account(name), 'name': name, 'properties': []})
                    return self.reply(204)
                self.reply(404, {'error': 'NotFound'})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
                if self.path != '/session/minecraft/join':
                    return self.reply(404, {'error': 'NotFound'})
                # The test client names its account instead of its id.
                with services.lock:
                    services.joined[body.get('selectedProfile')] = body.get('serverId')
                self.reply(204)

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.server.server_address[1]}'
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def discovery(self):
        def endpoints(**names):
            return {'endpoints': {k: {'uri': self.url + v} for k, v in names.items()}}
        return {'environment': 'mock', 'product': 'minecraft', 'discovery': {
            'authentication': endpoints(getPublicKeys='/publickeys'),
            'session': endpoints(join='/session/minecraft/join', verify='/session/minecraft/hasJoined',
                                 getProfileById='/session/minecraft/profile/{profileId}'),
            'player': {'endpoints': {}},
            'profiles': {'endpoints': {'getTexture': {'validUris': [self.url + '/texture/{textureId}']},
                                       'getByName': {'uri': self.url + '/users/profiles/minecraft/{name}'}}},
            'telemetry': {'endpoints': {}},
        }}

    def stop(self):
        self.server.shutdown()


def player_id(name, scenario):
    """The player's id: the account's online, UUIDUtil.createOfflinePlayerUUID offline."""
    if scenario == 'online':
        return account(name)
    digest = bytearray(hashlib.md5(f'OfflinePlayer:{name}'.encode()).digest())
    digest[6] = digest[6] & 0x0F | 0x30
    digest[8] = digest[8] & 0x3F | 0x80
    return digest.hex()


def dashed(u):
    return f'{u[:8]}-{u[8:12]}-{u[12:16]}-{u[16:20]}-{u[20:]}'


def status(port):
    """The status response's JSON (ServerStatus)."""
    def packet(body):
        return lab.varint(len(body)) + body

    def read_varint(s):
        value = 0
        for i in range(5):
            b = s.recv(1)[0]
            value |= (b & 0x7F) << (7 * i)
            if not b & 0x80:
                return value
    with socket.create_connection(('127.0.0.1', port), timeout=10) as s:
        host = b'127.0.0.1'
        s.sendall(packet(b'\x00' + lab.varint(777) + lab.varint(len(host)) + host + port.to_bytes(2, 'big') + b'\x01'))
        s.sendall(packet(b'\x00'))
        length = read_varint(s)
        data = b''
        while len(data) < length:
            data += s.recv(length - len(data))
    text = data[1:]
    n, shift, i = 0, 0, 0
    while True:
        b = text[i]
        n |= (b & 0x7F) << shift
        shift += 7
        i += 1
        if not b & 0x80:
            break
    return json.loads(text[i:i + n])


# The servers' lines about chat, with their level. The thread is left out:
# vanilla handles acknowledgements on its network threads, named per platform.
CHAT_LINE = re.compile(r'^\[\d\d:\d\d:\d\d\] \[([^\]]+)\]: (.*(?:<\w+>|\[Not Secure\]|\[\w+\] |\* \w+ |secure chat|'
                       r'signable|Signed command|acknowledgement|profile key|chat session|expired chat|lost connection|'
                       r'Disconnecting).*)$')


def chat_lines(lines):
    out = []
    for line in lines:
        if m := CHAT_LINE.match(line):
            text = re.sub(r'/127\.0\.0\.1:\d+', '/127.0.0.1:_', m.group(2))
            out.append(f'[{m.group(1).rsplit("/", 1)[-1]}] {text}')
    return out


def run_side(label, scenario, command, directory, client):
    port = lab.free_port()
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir(parents=True)
    props = dict(PROPERTIES, **SCENARIOS[scenario], **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    # Alice and Carol are operators (/say needs it), by their ids.
    ops = [{'uuid': dashed(player_id(name, scenario)), 'name': name, 'level': 4, 'bypassesPlayerLimit': False}
           for name in ('Alice', 'Carol')]
    (directory / 'ops.json').write_text(json.dumps(ops, indent=2))
    server = Server(command, directory, f'{label}-{scenario}')
    try:
        server.wait_for(r'Done \(', timeout=600)
        # A server can drop a ping in its first moments (the network thread still starting): try again.
        for attempt in range(5):
            try:
                enforces = status(port).get('enforcesSecureChat', False)
                break
            except (OSError, IndexError):
                if attempt == 4:
                    raise
                time.sleep(2)
        out = directory.parent / f'{label}-{scenario}.txt'
        client(port, scenario, out)
        # Carol leaves last: stopping before the server saw her go would log another reason.
        server.wait_for(r'Carol lost connection', timeout=30)
        transcript = [f'status enforcesSecureChat {enforces}'] + out.read_text(encoding='utf-8').splitlines()
    finally:
        server.stop()
    # A player the server disconnects (the transcript has the reason it sent, compared there) is logged as
    # "lost connection: <that reason>", or as "Disconnected" when vanilla's network thread closed the connection
    # before its disconnect(reason) ran: a race in vanilla, so the log's reason is left out for them.
    kicked = {m.group(1) for line in transcript if (m := re.match(r'(\w+) disconnected ', line))}
    log = [re.sub(r'^(\[\w+\] (\w+) lost connection: ).*', lambda m: m.group(1) + '(the reason sent)'
                  if m.group(2) in kicked else m.group(0), line) for line in chat_lines(server.lines)]
    return transcript, log


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/SecureChatClient.java'
    oh.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-sourcepath',
                   lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                  build, 'compile-java')
    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], build,
                  'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    java = ['java', '-Xmx1g', '-Duser.timezone=UTC', '-cp', str(classes) + os.pathsep + dependencies,
            'dev.horizonlab.parity.SecureChatClient']
    oh.run_logged([*java, 'keys', build], build, 'keys')
    services = Services(json.loads((build / 'publickeys.json').read_text()))
    discovery = services.url + '/minecraft/client'
    os.environ.update(lab.game_env(HORIZON_API_DISCOVERY=discovery))
    servers = {
        'vanilla': ['java', '-Xmx2g', '-Duser.timezone=UTC', f'-Dminecraft.api.discovery.host={discovery}',
                    '-cp', dependencies, 'net.minecraft.server.Main', 'nogui'],
        'horizon': [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui'],
    }

    def client(port, scenario, out):
        oh.run_logged([*java, '127.0.0.1', port, build / 'services-private.der', out, scenario, services.url],
                      build, out.stem, timeout=300)

    differences = []
    try:
        for scenario in SCENARIOS:
            results = {}
            for label, command in servers.items():
                results[label] = run_side(label, scenario, command, build / f'{label}-{scenario}', client)
                transcript, log = results[label]
                (build / f'{label}-{scenario}-all.txt').write_text('\n'.join(transcript + ['--- log'] + log) + '\n',
                                                                   encoding='utf-8')
            import difflib
            for part, index in (('transcript', 0), ('log', 1)):
                diff = [l for l in difflib.unified_diff(results['vanilla'][index], results['horizon'][index],
                                                        'vanilla', 'horizon', n=0, lineterm='')
                        if l[:1] in '+-' and l[:3] not in ('+++', '---')]
                if diff:
                    differences.append(f'{scenario} {part}:\n  ' + '\n  '.join(diff[:60]))
            errors = [l for l in results['vanilla'][0] if l.startswith('error')]
            if errors:
                differences.append(f'{scenario}: the vanilla run failed: {errors}')
    finally:
        services.stop()
    if differences:
        raise AssertionError('\n'.join(differences) + f'\n(transcripts in {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        scope='Secure chat against vanilla\'s dedicated server: chat sessions validated against mock services keys, '
              'signed chat with last seen and the signature cache, unsigned chat, /say /me /msg with signed and '
              'unsigned arguments, mismatched and forged signatures, the broken chain, a bad acknowledgement; '
              'enforce-secure-profile in online mode (status, login, refusals, a borrowed key); the chat log lines.'))
    print('secure chat: transcripts and chat log lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
