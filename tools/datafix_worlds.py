#!/usr/bin/env python3
"""Make sample worlds with older vanilla servers, for the DataFixer harness.

For each listed version, the official server jar (cached under
.local/datafix/servers/VERSION/server.jar, beside the shared vanilla data) generates a default world around spawn,
gets a scene of block entities, entities and items in that version's own save
format (tools/datafix_scene.py), and a bot player joins (with its inventory filled)
so that player data exists. The saved world is kept in datafix/worlds/VERSION beside the shared vanilla data
(.local/datafix in a plain checkout), where every worktree finds it.

  datafix_worlds.py [--force] [VERSION...]      (default: every version below)
"""
import argparse
import hashlib
import json
import queue
import re
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.request
import zipfile
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import datafix_scene as scene  # noqa: E402
import datafix_scene_12 as scene12  # noqa: E402
import lab  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
LOCAL = ROOT / '.local' / 'datafix'
# Old server jars and their worlds are inputs, not outputs: keep them beside the
# vanilla data, which parallel worktrees share (.local/minecraft links there).
SHARED = (ROOT / '.local' / 'minecraft').resolve().parent / 'datafix'
# Worlds saved before 1.17 are only read by branches that carry the fixes for them: until those are
# integrated they stay in the worktree's own .local, not in the folder every worktree's harness reads.
LEGACY = {'1.8.9', '1.9.4', '1.10.2', '1.11.2', '1.12.2', '1.13.2', '1.14.4', '1.15.2', '1.16.1', '1.16.5'}


def worlds_dir(name):
    return LOCAL / 'worlds-pre-1.17' if name.split('-')[0] in LEGACY else SHARED / 'worlds'


VERSIONS = ['1.8.9', '1.9.4', '1.10.2', '1.11.2', '1.12.2', '1.13.2', '1.14.4', '1.15.2', '1.16.1', '1.16.5', '1.17.1', '1.18.1', '1.18.2', '1.19', '1.19.2', '1.19.3', '1.19.4', '1.20.1', '1.20.2', '1.20.4', '1.20.5', '1.20.6', '1.21.1',
            '1.21.3', '1.21.4', '1.21.5', '1.21.6', '1.21.8', '1.21.9', '1.21.10', '1.21.11', '26.1', '26.1.2', '26.2']
# Extra worlds of one version with another world type, for the level.dat generator settings:
# name -> (version, server.properties). They get no scene, only a joined player.
VARIANTS = {
    '1.13.2-flat': ('1.13.2', {'level-type': 'flat', 'generator-settings':
                               '3;minecraft:bedrock,2*minecraft:dirt,minecraft:grass_block;1;village,oceanmonument,mansion(distance=30)'}),
    '1.14.4-flat': ('1.14.4', {'level-type': 'flat', 'generator-settings':
                               '3;minecraft:bedrock,3*minecraft:stone,minecraft:grass_block;2;stronghold(distance=20 count=5),village(distance=12),'
                               'oceanmonument(spacing=10 separation=4)'}),
    '1.14.4-largebiomes': ('1.14.4', {'level-type': 'largebiomes'}),
    '1.14.4-amplified': ('1.14.4', {'level-type': 'amplified'}),
    '1.13.2-debug': ('1.13.2', {'level-type': 'debug_all_block_states'}),
    '1.14.4-buffet': ('1.14.4', {'level-type': 'buffet', 'generator-settings':
                                 '{"chunk_generator":{"type":"minecraft:surface"},"biome_source":{"type":"minecraft:fixed","options":'
                                 '{"biomes":["minecraft:desert"]}}}'}),
}
MANIFEST = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
LOCK = json.loads((ROOT / 'research-lock.json').read_text())['java_runtimes']
BOT = 'Upgrader'
UUID_BASE = 0x6000


def version_meta(version):
    path = SHARED / 'servers' / version / 'version.json'
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        manifest = json.load(urllib.request.urlopen(MANIFEST))
        entry = next(v for v in manifest['versions'] if v['id'] == version)
        data = urllib.request.urlopen(entry['url']).read()
        if hashlib.sha1(data).hexdigest() != entry['sha1']:
            sys.exit(f'{version}: version metadata checksum mismatch')
        path.write_bytes(data)
    return json.loads(path.read_text())


def server_jar(version):
    """The server jar, its DataVersion (None before 1.9: no DataVersion) and protocol."""
    path = SHARED / 'servers' / version / 'server.jar'
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        download = version_meta(version)['downloads']['server']
        data = urllib.request.urlopen(download['url']).read()
        if hashlib.sha1(data).hexdigest() != download['sha1']:
            sys.exit(f'{version}: server.jar checksum mismatch')
        path.write_bytes(data)
    jar = zipfile.ZipFile(path)
    if 'version.json' in jar.namelist():
        info = json.loads(jar.read('version.json'))
        return path, info['world_version'], info['protocol_version']
    return path, OLD_VERSIONS[version][0], OLD_VERSIONS[version][1]


# DataVersion and protocol of servers whose jar has no version.json (before 1.14).
OLD_VERSIONS = {
    '1.13.2': (1631, 404), '1.12.2': (1343, 340), '1.11.2': (922, 316), '1.10.2': (512, 210),
    '1.9.4': (184, 110), '1.8.9': (None, 47),
}


def platform():
    import platform as p
    if sys.platform == 'darwin':
        return 'mac-os-arm64' if p.machine() == 'arm64' else 'mac-os'
    return 'linux'


def java(version):
    """The java executable for a server version: Mojang's runtime of the version's
    component (pinned in research-lock.json), installed beside the shared data."""
    meta = version_meta(version)
    component = meta.get('javaVersion', {}).get('component', 'jre-legacy')
    if component in ('java-runtime-delta', 'java-runtime-epsilon'):
        return 'java'  # 1.20.5 on: the JDK that runs 26.3 runs them
    runtimes = LOCK[platform()]
    if component not in runtimes:
        component = 'java-runtime-gamma'
    pin = runtimes[component]
    home = SHARED / 'java' / f'{component}-{platform()}'
    stamp = home / '.complete'
    if not stamp.exists() or stamp.read_text() != pin['manifest_sha1']:
        install_runtime(pin, home)
        stamp.write_text(pin['manifest_sha1'])
    exe = next(home.glob('**/bin/java'))
    return str(exe)


def install_runtime(pin, home):
    data = urllib.request.urlopen(pin['manifest_url']).read()
    if hashlib.sha1(data).hexdigest() != pin['manifest_sha1']:
        sys.exit(f'{pin["manifest_url"]}: checksum mismatch')
    shutil.rmtree(home, ignore_errors=True)
    links = []
    for name, entry in sorted(json.loads(data)['files'].items()):
        target = home / name
        if entry['type'] == 'directory':
            target.mkdir(parents=True, exist_ok=True)
        elif entry['type'] == 'link':
            links.append((target, entry['target']))
        else:
            raw = entry['downloads']['raw']
            body = urllib.request.urlopen(raw['url']).read()
            if hashlib.sha1(body).hexdigest() != raw['sha1']:
                sys.exit(f'{name}: checksum mismatch')
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(body)
            if entry.get('executable'):
                target.chmod(0o755)
    for target, to in links:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.symlink_to(to)


class Server:
    def __init__(self, directory, jar, port, packs=(), java='java', props_override=None):
        directory.mkdir(parents=True, exist_ok=True)
        self.directory = directory
        (directory / 'eula.txt').write_text('eula=true\n')
        props = {'server-port': port, 'online-mode': 'false', 'level-name': 'world', 'level-seed': '4242',
                 'view-distance': '4', 'simulation-distance': '4', 'spawn-protection': '0', 'sync-chunk-writes': 'true',
                 'enable-rcon': 'false', 'enable-query': 'false', 'max-tick-time': '-1'}
        if packs:
            props['initial-enabled-packs'] = ','.join(['vanilla', *packs])
        props.update(props_override or {})
        (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
        self.process = subprocess.Popen([java, '-Xmx2G', '-jar', str(jar), 'nogui'], cwd=directory,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.lines = []
        self.queue = queue.Queue()
        self.counter = 0
        threading.Thread(target=self._pump, daemon=True).start()
        self.wait('Done (', 600)

    def _pump(self):
        for line in self.process.stdout:
            line = line.rstrip('\n')
            self.lines.append(line)
            self.queue.put(line)

    def wait(self, pattern, timeout=60):
        end = time.time() + timeout
        seen = []
        while time.time() < end:
            try:
                line = self.queue.get(timeout=1)
            except queue.Empty:
                if self.process.poll() is not None:
                    break
                continue
            seen.append(line)
            if pattern in line:
                return seen
        raise TimeoutError(f'no "{pattern}" in {timeout}s; last lines: {seen[-5:]}')

    def run(self, command, timeout=60):
        self.counter += 1
        marker = f'@@{self.counter}@@'
        self.process.stdin.write(command + '\nsay ' + marker + '\n')
        self.process.stdin.flush()
        out = self.wait(marker, timeout)
        return [re.sub(r'^\[[0-9:]+\] \[[^\]]*\]: ', '', l) for l in out[:-1]]

    def stop(self):
        try:
            self.process.stdin.write('stop\n')
            self.process.stdin.flush()
            self.process.wait(120)
        except Exception:
            self.process.kill()


def failed(lines):
    return [l for l in lines if re.search(r'(Unknown|Expected|Incorrect|Invalid|Could not|Unable|Failed|That block|'
                                          r'error|Malformed|Can\'t)', l, re.I)]


# ------------------------------------------------------------------ minimal bot

def mcstring(s):
    data = s.encode()
    return lab.varint(len(data)) + data


class Bot:
    """Logs in (offline mode) with the given protocol and stays until told to leave."""

    def __init__(self, port, protocol, name):
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=30)
        self.threshold = -1
        self.buffer = b''
        self.protocol = protocol
        self.stop = threading.Event()
        self.send(0, lab.varint(protocol) + mcstring('127.0.0.1') + struct.pack('>H', port) + lab.varint(2))
        if protocol <= 758:  # 1.18.2: the name only
            self.send(0, mcstring(name))
        elif protocol == 759:  # 1.19: no signature data
            self.send(0, mcstring(name) + b'\x00')
        elif protocol == 760:  # 1.19.2: no signature data, optional UUID
            self.send(0, mcstring(name) + b'\x00' + b'\x01' + lab.offline_uuid(name).bytes)
        elif protocol <= 763:  # 1.19.3 - 1.20.1: optional UUID
            self.send(0, mcstring(name) + b'\x01' + lab.offline_uuid(name).bytes)
        else:
            self.send(0, mcstring(name) + lab.offline_uuid(name).bytes)
        while True:
            pid, data = self.receive()
            if pid == 3:
                self.threshold = lab.parse_varint(data, 0)[1]
            elif pid == 2:
                break
            elif pid == 0:
                raise RuntimeError(f'login refused: {data!r}')
        if protocol >= 764:
            self.send(3)  # login acknowledged -> configuration
            self.configure()
        threading.Thread(target=self.idle, daemon=True).start()

    def configure(self):
        old = self.protocol < 766  # 1.20.2 - 1.20.4 packet ids
        finish, keep_alive, ping, disconnect = (2, 3, 4, 1) if old else (3, 4, 5, 2)
        while True:
            pid, data = self.receive()
            if pid == finish:
                self.send(2 if old else 3)
                return
            if pid == keep_alive:
                self.send(3 if old else 4, data)
            elif pid == ping:
                self.send(4 if old else 5, data)
            elif pid == disconnect:
                raise RuntimeError(f'disconnected during configuration: {data!r}')
            elif not old and self.known_packs(data):
                self.send(7, lab.varint(0))  # claim no packs: the server sends full registries

    @staticmethod
    def known_packs(data):
        try:
            pos, count = lab.parse_varint(data, 0)
            if count < 1:
                return False
            pos, n = lab.parse_varint(data, pos)
            return data[pos:pos + n] == b'minecraft'
        except IndexError:
            return False

    def idle(self):
        self.sock.settimeout(1.0)
        while not self.stop.is_set():
            try:
                self.receive()
            except socket.timeout:
                continue
            except (EOFError, OSError):
                break

    def leave(self):
        self.stop.set()
        time.sleep(1.5)
        self.sock.close()

    def read_exact(self, n):
        while len(self.buffer) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError('server closed the connection')
            self.buffer += chunk
        out, self.buffer = self.buffer[:n], self.buffer[n:]
        return out

    def read_varint(self):
        value = shift = 0
        while True:
            b = self.read_exact(1)[0]
            value |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                return value

    def send(self, packet_id, payload=b''):
        body = lab.varint(packet_id) + payload
        if self.threshold >= 0:
            body = lab.varint(len(body)) + zlib.compress(body) if len(body) >= self.threshold else lab.varint(0) + body
        self.sock.sendall(lab.varint(len(body)) + body)

    def receive(self):
        length = self.read_varint()
        body = self.read_exact(length)
        if self.threshold >= 0:
            pos, size = lab.parse_varint(body, 0)
            body = zlib.decompress(body[pos:]) if size else body[pos:]
        pos, packet_id = lab.parse_varint(body, 0)
        return packet_id, body[pos:]


# ------------------------------------------------------------------ world

def uuid_nbt(i):
    return f'[I;{UUID_BASE},0,0,{i}]'


def uuid_fields(data_version, i):
    """The UUID of a summoned entity: an int array since 1.16, two longs before."""
    if data_version >= 2514:
        return f'UUID:{uuid_nbt(i)}'
    return f'UUIDMost:{UUID_BASE << 32}L,UUIDLeast:{i + 1}L'


def build_scene(server, run, Bot, port, protocol, props, era, data_version):
    """The scene of the generic generator; returns the bot that stays joined."""
    if not props:  # variants only differ in their world type
        run('forceload add -48 -48 79 79')
        run('fill 0 99 0 47 99 47 minecraft:stone')
        run('fill 0 100 0 47 112 47 minecraft:air')
        for i, (block, nbt) in enumerate(scene.block_entities(era, data_version)):
            x, z = 1 + 3 * (i % 15), 1 + 3 * (i // 15)
            run(f'setblock {x} 100 {z} minecraft:{block}{nbt}')
        for i, (kind, extra) in enumerate(scene.entities(era, data_version)):
            x, z = 1.5 + 3 * (i % 15), 25.5 + 3 * (i // 15)
            fields = f'{uuid_fields(data_version, i)},NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b'
            run(f'summon minecraft:{kind} {x} 100 {z} {{{fields}' + (',' + extra if extra else '') + '}')
        for i, block in enumerate(scene.blocks(data_version)):
            run(f'setblock {1 + 3 * (i % 15)} 100 {19 + 3 * (i // 15)} minecraft:{block}')
    if not props:
        run('setworldspawn 20 101 20')
    bot = Bot(port, protocol, BOT)
    time.sleep(3)
    for command in scene.player_commands(era, BOT, data_version):
        run(command)
    time.sleep(3)
    for command, wait in ([] if props else scene.world_commands(data_version)):
        run(command)
        time.sleep(wait)
    run('save-all flush')
    time.sleep(2)
    return bot


def build(name, force=False):
    version, props = VARIANTS.get(name, (name, None))
    jar, data_version, protocol = server_jar(version)
    data_version = data_version or 0   # 1.8 saves have no DataVersion
    target = worlds_dir(name) / name
    if target.exists() and not force:
        print(f'{version}: kept {target}')
        return []
    era = scene.era(data_version)
    work = LOCAL / 'gen' / name
    shutil.rmtree(work, ignore_errors=True)
    port = lab.free_port()
    server = Server(work, jar, port, scene.experiments(data_version), java(version), props)
    problems = []

    def run(command):
        out = server.run(command)
        bad = failed(out)
        if bad:
            problems.append((command[:160], bad[:2]))
        return out

    if data_version < 1631 and not props:   # 1.12 and older: tools/datafix_scene_12.py builds the scene and saves the world itself
        bot = scene12.populate(server, Bot, port, protocol, BOT, problems)
    else:
        bot = build_scene(server, run, Bot, port, protocol, props, era, data_version)
    bot.leave()
    time.sleep(2)
    server.stop()
    if data_version < 1631 and not props:
        scene12.fixtures(work / 'world')   # the few statistics a passive bot cannot cause, written by hand
    # Copy aside, then rename: an interrupted copy must not look like a finished world.
    partial = target.with_name(target.name + '.partial')
    shutil.rmtree(partial, ignore_errors=True)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(work / 'world', partial)
    (partial / 'datafix-source.json').write_text(json.dumps(
        {'version': version, 'data_version': data_version, 'problems': problems}, indent=1))
    shutil.rmtree(target, ignore_errors=True)
    partial.rename(target)
    for command, out in problems:
        print(f'{version}: command failed: {command}\n   {out}')
    print(f'{version}: DataVersion {data_version}, world in {target}')
    return problems


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('versions', nargs='*')
    p.add_argument('--force', action='store_true')
    args = p.parse_args()
    for version in args.versions or [*VERSIONS, *VARIANTS]:
        build(version, args.force)
        if version == '1.12.2':
            scene12.build_flat(sys.modules[__name__], args.force)   # and the flat world 1.12.2-flat


if __name__ == '__main__':
    main()
