#!/usr/bin/env python3
"""Horizon's client joins vanilla's online-mode server, with no real account.

A small mock of the Minecraft services (dev only: the discovery document and
the session service's join and hasJoined endpoints) stands in for Mojang's.
The pinned vanilla 26.3 dedicated server runs in online mode and finds the
mock through authlib's own override, `-Dminecraft.api.discovery.host`;
Horizon's client finds it through `HORIZON_API_DISCOVERY` and signs in with a
session file holding a mock token, as the launcher hands it over.

The client must answer the server's encryption request: generate the shared
secret, hash the server id with it and the server's public key, join through
the mock, RSA-wrap the secret and the challenge for the server, then encrypt
the connection with AES/CFB8. The server decrypts the secret with its private
key, computes the hash itself and asks the mock whether the player joined
with that hash: so vanilla's own code checks the hash, the RSA wrapping and
the cipher. Then the client plays as in the dedicated-server harness (stands
in the world with its chunks, chats, runs /list, leaves).

Also checked, with vanilla's messages:
- a token the session service rejects ("Failed to log in: Invalid session");
- a join the session service forgets (the server: "Failed to verify username!");
- no account at all (the client refuses before joining).

Chat is unsigned here: the mock serves no Mojang public keys, so the server
can't validate profile keys and doesn't enforce secure chat.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import threading
import urllib.parse

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port
except ImportError:
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port

NAME = 'online-mode-parity'
PLAYER = 'Tester'
PLAYER_UUID = '5f3c1e6a2b9d4c7e8a1f0b2c3d4e5f60'
GOOD, FORGET, BAD = 'mock-token-good', 'mock-token-forget', 'mock-token-rejected'
PROPERTIES = {
    'online-mode': 'true',
    'white-list': 'false',
    'level-seed': '12345',
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
    'max-players': '5',
    'pause-when-empty-seconds': '-1',
}


def dashed(u):
    return f'{u[:8]}-{u[8:12]}-{u[12:16]}-{u[16:20]}-{u[20:]}'


class Services:
    """The mock session service. Records what was asked, never the tokens."""

    def __init__(self):
        self.joined = {}
        self.log = []
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
                if url.path == '/session/minecraft/hasJoined':
                    with services.lock:
                        ok = services.joined.get(query.get('username')) == query.get('serverId')
                        services.log.append(('verify', query.get('username'), query.get('serverId'), ok))
                    if ok:
                        return self.reply(200, {'id': PLAYER_UUID, 'name': query['username'], 'properties': []})
                    return self.reply(204)
                self.reply(404, {'error': 'NotFound', 'path': url.path})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
                if self.path != '/session/minecraft/join':
                    return self.reply(404, {'error': 'NotFound'})
                token, profile, server_id = body.get('accessToken'), body.get('selectedProfile'), body.get('serverId')
                accepted = token in (GOOD, FORGET) and profile == PLAYER_UUID
                with services.lock:
                    services.log.append(('join', profile, server_id, accepted))
                    if token == GOOD and accepted:
                        services.joined[PLAYER] = server_id
                if not accepted:
                    return self.reply(403, {'error': 'ForbiddenOperationException', 'errorMessage': 'Invalid token'})
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
            'profiles': {'endpoints': {'getTexture': {'validUris': [self.url + '/texture/{textureId}']}}},
            'telemetry': {'endpoints': {}},
        }}

    def take_log(self):
        with self.lock:
            log, self.log = self.log, []
            return log

    def stop(self):
        self.server.shutdown()


def session_file(directory, token):
    """What the launcher writes: owner-only, read once by the game."""
    path = directory / 'session.json'
    path.unlink(missing_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f:
        json.dump({'name': PLAYER, 'uuid': dashed(PLAYER_UUID), 'access_token': token, 'user_type': 'msa'}, f)
    return path


def run_client(binary, build, port, label, token, discovery):
    directory = build / f'client-{label}'
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir()
    report = directory / 'report.json'
    env = dict(os.environ, HORIZON_NO_AUDIO='1', HORIZON_NET_REPORT=str(report), HORIZON_API_DISCOVERY=discovery)
    command = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--render-distance', 2,
               '--server', f'127.0.0.1:{port}', '--name', PLAYER]
    if token:
        command += ['--session-file', session_file(directory, token)]
    with (build / f'client-{label}.log').open('w') as log:
        subprocess.run([str(c) for c in command], cwd=directory, env=env, stdout=log, stderr=subprocess.STDOUT,
                       timeout=300, check=True)
    got = json.loads(report.read_text())
    got['session file left'] = (directory / 'session.json').exists()
    return got


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oh.run_logged(['cargo', 'build', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'], build,
                  'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))

    services = Services()
    discovery = services.url + '/minecraft/client'
    port = free_port()
    directory = build / 'server'
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    command = ['java', '-Xmx2g', '-Duser.timezone=UTC', f'-Dminecraft.api.discovery.host={discovery}',
               '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    server = Server(command, directory, 'vanilla-online')
    checks = []

    def check(label, actual, expected):
        checks.append(f'{label}: ' + ('ok' if actual == expected else f'expected {expected!r}, got {actual!r}'))

    try:
        server.wait_for(r'Done \(', timeout=600)
        services.take_log()

        # 1. Signed in: the full encrypted handshake, then play.
        got = run_client(binary, build, port, 'signed-in', GOOD, discovery)
        server.wait_for(r'Tester left the game', timeout=30)
        log = services.take_log()
        joins = [e for e in log if e[0] == 'join']
        verifies = [e for e in log if e[0] == 'verify']
        check('signed-in: connected throughout', got['error'], None)
        check('signed-in: joined the session service once', [(e[1], e[3]) for e in joins], [(PLAYER_UUID, True)])
        check('signed-in: server verified the same hash',
              [(e[1], e[2], e[3]) for e in verifies], [(PLAYER, joins[0][2] if joins else None, True)])
        check('signed-in: own chat shown', any('<Tester> hello from the horizon client' in c for c in got['chat']), True)
        check('signed-in: /list reply shown',
              any('There are 1 of a max of 5 players online: Tester' in c for c in got['chat']), True)
        check('signed-in: chunks received', got['chunks'] > 0, True)
        check('signed-in: session file deleted', got['session file left'], False)
        lines = '\n'.join(server.lines)
        check('signed-in: server log', [k for k in [f'UUID of player Tester is {dashed(PLAYER_UUID)}',
                                                    'Tester joined the game',
                                                    '<Tester> hello from the horizon client',
                                                    'Tester left the game'] if k in lines], [
            f'UUID of player Tester is {dashed(PLAYER_UUID)}', 'Tester joined the game',
            '<Tester> hello from the horizon client', 'Tester left the game'])

        # 2. A token the session service rejects: vanilla's message, no key sent.
        mark = len(server.lines)
        got = run_client(binary, build, port, 'rejected', BAD, discovery)
        check('rejected token: client message', got['error'],
              'disconnected: Failed to log in: Invalid session (Try restarting your game and the launcher)')
        check('rejected token: no verification', [e[0] for e in services.take_log()], ['join'])

        # 3. The session service forgets the join: the server refuses the player.
        got = run_client(binary, build, port, 'forgotten', FORGET, discovery)
        server.wait_for(r'Failed to verify username', timeout=30)
        check('forgotten join: client message', got['error'], 'disconnected: Failed to verify username!')
        check('forgotten join: server verified and refused',
              [(e[0], e[3]) for e in services.take_log()], [('join', True), ('verify', False)])

        # 4. No account: refused by the client before anything is sent to the services.
        got = run_client(binary, build, port, 'no-account', None, discovery)
        check('no account: client message', got['error'] or '', 'disconnected: This server is in online mode. '
              'Sign in with your Microsoft account in the Horizon launcher to join it.')
        check('no account: no service calls', services.take_log(), [])
        check('no player joined after the first', 'joined the game' in '\n'.join(server.lines[mark:]), False)
    finally:
        server.stop()
        services.stop()

    for line in checks:
        print(line)
    failed = [c for c in checks if not c.endswith(': ok')]
    if failed:
        raise AssertionError(f'{len(failed)} of {len(checks)} checks failed (logs in {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0], checks=len(checks),
        scope='Online-mode login against vanilla\'s dedicated server and a mock session service: server hash, '
              'RSA key packet, AES/CFB8, session join and verify, play; rejected token, forgotten join, no account.'))
    print(f'online mode: {len(checks)} checks ok')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
