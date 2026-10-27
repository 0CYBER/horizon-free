#!/usr/bin/env python3
"""usercache.json and the console's player names, against vanilla's dedicated server.

The console finds a player by name through the user cache (CachedUserNameToIdResolver): a name it holds
in any case, else the profile service's `getByName` (asked with the lower-case name), else, in offline
mode, the offline player of that lower-case name; what it finds is cached for a month and the file
rewritten, most recently used first. A small mock of the Minecraft services (dev only: the discovery
document and `getByName`, which knows one account) stands in for Mojang's; vanilla finds it through
`-Dminecraft.api.discovery.host`, Horizon through `HORIZON_API_DISCOVERY`.

Each server starts in offline mode in a folder holding a usercache.json with a live entry, an expired
one, entries without a date, with a bad date, without an id, and not an object. The console then
whitelists, ops, deops and bans players: the cached one in another case, the expired one, the account the
service knows, names it doesn't know, a name too long to ask about. The server restarts in online mode,
where a name nobody knows is no player at all, and stops. Compared exactly: the console's replies, the
names asked of the service in order, and usercache.json, whitelist.json, ops.json and
banned-players.json after each run (the dates the servers wrote are placeholders).

Players who join are added by the dedicated-server harness, which compares usercache.json too.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.parse

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port

NAME = 'user-cache-parity'
PROPERTIES = {
    'online-mode': 'false',
    'level-seed': '12345',
    'generate-structures': 'false',
    'view-distance': '2',
    'simulation-distance': '2',
    'spawn-protection': '0',
    'pause-when-empty-seconds': '-1',
}
NOTCH = '069a79f444e94726a5befca90e38aaf5'
SEEDED = [
    {'uuid': '853c80ef-3c37-49fd-aa49-938b674adae6', 'name': 'Seeded', 'expiresOn': '2099-01-01 00:00:00 +0000'},
    {'uuid': '61699b2e-d327-4a01-9f1e-0ea8c3f06bc6', 'name': 'Expired', 'expiresOn': '2001-02-03 04:05:06 +0000'},
    {'uuid': '00000000-0000-0000-0000-000000000001', 'name': 'NoDate'},
    {'uuid': '00000000-0000-0000-0000-000000000002', 'name': 'BadDate', 'expiresOn': 'tomorrow'},
    {'name': 'NoId', 'expiresOn': '2099-01-01 00:00:00 +0000'},
    'not an object',
    {'uuid': '00000000-0000-0000-0000-000000000003', 'name': 'Later', 'expiresOn': '2098-05-06 07:08:09 +0100'},
]
# (console command, the reply it waits for)
OFFLINE = [
    ('whitelist add SEEDED', r'System chat: (Added|Player is already)'),
    ('whitelist add Notch', r'System chat: (Added|Player is already)'),
    ('op expired', r'System chat: (Made|Nothing changed)'),
    ('ban Nobody_Here', r'System chat: (Banned|Nothing changed)'),
    ('whitelist add NameMuchTooLongForMinecraft', r'System chat: (Added|Player is already)'),
    ('deop NOTCH', r'System chat: (Made|Nothing changed)'),
    ('whitelist add later', r'System chat: (Added|Player is already)'),
    ('whitelist remove notch', r'System chat: (Removed|Player is not)'),
]
ONLINE = [
    ('whitelist add Somebody_Else', r'System chat: (Added|That player does not exist)'),
    ('whitelist add nobody_here', r'System chat: (Added|Player is already)'),
]
FILES = ['usercache.json', 'whitelist.json', 'ops.json', 'banned-players.json']


class Services:
    """The mock discovery document and profile lookup by name."""

    def __init__(self):
        self.asked = []
        self.lock = threading.Lock()
        services = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, status, body):
                data = json.dumps(body).encode()
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                url = urllib.parse.urlparse(self.path)
                if url.path == '/minecraft/client':
                    return self.reply(200, services.discovery())
                if url.path.startswith('/profiles/name/'):
                    name = urllib.parse.unquote(url.path.removeprefix('/profiles/name/'))
                    with services.lock:
                        services.asked.append(name)
                    if name.lower() == 'notch':
                        return self.reply(200, {'id': NOTCH, 'name': 'Notch'})
                    return self.reply(404, {'path': url.path, 'errorMessage': f"Couldn't find any profile with name {name}"})
                self.reply(404, {'error': 'NotFound', 'path': url.path})

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
            'player': endpoints(),
            'profiles': endpoints(getByName='/profiles/name/{name}'),
            'telemetry': endpoints(),
        }}

    def take(self):
        with self.lock:
            asked, self.asked = self.asked, []
            return asked

    def stop(self):
        self.server.shutdown()


def write_properties(directory, online):
    props = dict(PROPERTIES, **{'server-port': str(free_port()), 'online-mode': str(online).lower()})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                 encoding='iso-8859-1')


def files(directory):
    out = []
    for name in FILES:
        path = directory / name
        if not path.exists():
            out.append(f'{name}: missing')
            continue
        text = path.read_text(encoding='utf-8')
        # The dates written now: a month on for the cache, the ban's creation.
        text = re.sub(r'"(expiresOn|created)": ?"20[2-3]\d-[^"]*"', r'"\1":"<now>"', text)
        out.append(f'{name}: {text}')
    return out


def wait_after(server, seen, pattern, timeout=60):
    """Waits for a line matching `pattern` after the first `seen` lines."""
    deadline = time.time() + timeout
    regex = re.compile(pattern)
    while not any(regex.search(l) for l in server.lines[seen:]):
        if time.time() > deadline or server.process.poll() is not None:
            raise AssertionError(f'{server.log_path.name}: no reply matching {pattern!r}:\n'
                                 + '\n'.join(server.lines[-20:]))
        time.sleep(0.1)


def run(label, command, build, services, steps, online, transcript):
    directory = build / label
    write_properties(directory, online)
    server = Server(command, directory, f'{label}-{"online" if online else "offline"}')
    try:
        server.wait_for(r'Done \(')
        services.take()
        for line, reply in steps:
            seen = len(server.lines)
            server.console(line)
            wait_after(server, seen, reply)
            replies = [l for l in server.lines[seen:] if 'System chat:' in l]
            transcript.append(f'> {line}')
            transcript += [re.sub(r'^\[[^]]*\] ', '', l) for l in replies]
            transcript.append(f'asked: {services.take()}')
    finally:
        server.stop()
    transcript += files(directory)


def run_side(label, command, build, services):
    directory = build / label
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    (directory / 'eula.txt').write_text('eula=true\n')
    (directory / 'usercache.json').write_text(json.dumps(SEEDED), encoding='utf-8')
    transcript = []
    run(label, command, build, services, OFFLINE, False, transcript)
    run(label, command, build, services, ONLINE, True, transcript)
    return transcript


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    services = Services()
    discovery = services.url + '/minecraft/client'
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', f'-Dminecraft.api.discovery.host={discovery}',
                      '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    os.environ['HORIZON_API_DISCOVERY'] = discovery
    try:
        vanilla = run_side('vanilla', vanilla_server, build, services)
        horizon = run_side('horizon', horizon_server, build, services)
    finally:
        services.stop()
    (build / 'vanilla.txt').write_text('\n'.join(vanilla) + '\n', encoding='utf-8')
    (build / 'horizon.txt').write_text('\n'.join(horizon) + '\n', encoding='utf-8')
    if vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'transcripts differ (see {build}/vanilla.txt and horizon.txt):\n' + diff[:6000])
    if not any('Notch' in l for l in vanilla if l.startswith('usercache.json')):
        raise AssertionError(f'the cache never held the looked-up account (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='usercache.json (load, expiry, most recently used order, Gson output) and the console finding '
              'players by name through it and the profile service, offline and online.'))
    print(f'user cache: {len(vanilla)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
