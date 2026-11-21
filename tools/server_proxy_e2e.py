#!/usr/bin/env python3
"""horizon-server's `[proxy]` on a real server, end to end (docs/hosting.md, #16251).

    python3 tools/server_proxy_e2e.py --bin-dir target/fast

`horizon-server run` supervises Horizon's dedicated server with `[proxy] enabled = true`: the supervisor listens
on the public port, the game on 127.0.0.1 behind it with Velocity forwarding. The player is a small client
written here (the protocol of 26.3: handshake, login, configuration, play), because the check is on one TCP
connection: a horizon-bots player cannot say whether its connection was the same one.

Steps, each checked:

1. The server list through the proxy is the game's own (its motd).
2. The client joins through the proxy, reaches play (the game's Login packet) and answers keep-alives.
3. It chats a command (`/give`, after the console ops it) and the game runs it for the player: the inventory.
4. The game saved the player under the UUID of the name: the proxy forwarded who the client is.
5. A client that connects to the game's own port directly is refused (forwarding is required).

Everything runs in a fresh temporary folder with fresh HORIZON_HOME and XDG folders and no audio. Logs go to
--out. Build first:
    cargo build --profile fast -p horizon-client -p horizon-server
"""
import argparse
import hashlib
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
import uuid as uuidlib
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PROTOCOL = 777
NAME = 'Prox'

# Packet ids of protocol 777 (crates/horizon-protocol/src/ids.rs).
HANDSHAKE, LOGIN_START, LOGIN_ACK = 0, 0, 3
CFG_FINISH, CFG_KEEP_ALIVE, CFG_PING, CFG_KNOWN_PACKS, CFG_DISCONNECT = 3, 4, 5, 15, 2
PLAY_KEEP_ALIVE_IN, PLAY_KEEP_ALIVE_OUT, PLAY_LOGIN, PLAY_DISCONNECT, PLAY_START_CONFIG = 45, 28, 50, 32, 120
PLAY_CHAT_COMMAND, PLAY_CONFIG_ACK = 7, 16


class Failed(Exception):
    pass


def check(condition, what):
    if not condition:
        raise Failed(what)
    print(f'ok: {what}')


def varint(n):
    out = b''
    n &= 0xFFFFFFFF
    while True:
        b, n = n & 0x7F, n >> 7
        out += bytes([b | 0x80]) if n else bytes([b])
        if not n:
            return out


def read_varint(buf, i=0):
    n = shift = 0
    while True:
        b = buf[i]
        i += 1
        n |= (b & 0x7F) << shift
        if not b & 0x80:
            return n, i
        shift += 7


def string(s):
    b = s.encode()
    return varint(len(b)) + b


def offline_uuid(name):
    md5 = bytearray(hashlib.md5(('OfflinePlayer:' + name).encode()).digest())
    md5[6] = (md5[6] & 0x0F) | 0x30
    md5[8] = (md5[8] & 0x3F) | 0x80
    return uuidlib.UUID(bytes=bytes(md5))


class Client:
    """A player on one TCP connection. `packets` is every (state, id) it received, in order; `closed` is set
    when the connection ended, with `reason` (a disconnect packet's payload, or the error)."""

    def __init__(self, port, name):
        self.name = name
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=60)
        self.local = self.sock.getsockname()
        self.threshold = -1
        self.state = 'login'
        self.packets = []
        self.closed = threading.Event()
        self.reason = None
        self.in_play = threading.Event()
        self.send(HANDSHAKE, varint(PROTOCOL) + string('localhost') + struct.pack('>H', port) + varint(2))
        self.send(LOGIN_START, string(name) + offline_uuid(name).bytes)
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def send(self, packet_id, payload=b''):
        body = varint(packet_id) + payload
        if self.threshold >= 0:
            body = varint(0) + body if len(body) < self.threshold else varint(len(body)) + zlib.compress(body)
        self.sock.sendall(varint(len(body)) + body)

    def _recv_exact(self, n):
        data = b''
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                raise EOFError('connection closed')
            data += chunk
        return data

    def _frame(self):
        length = shift = 0
        while True:
            b = self._recv_exact(1)[0]
            length |= (b & 0x7F) << shift
            if not b & 0x80:
                break
            shift += 7
        body = self._recv_exact(length)
        if self.threshold >= 0:
            size, i = read_varint(body)
            body = zlib.decompress(body[i:]) if size else body[i:]
        return read_varint(body)[0], body[read_varint(body)[1]:]

    def _read(self):
        try:
            while True:
                packet_id, payload = self._frame()
                self.packets.append((self.state, packet_id))
                self._handle(packet_id, payload)
        except (EOFError, OSError, zlib.error) as e:
            self.closed.set()
            self.reason = self.reason or f'{type(e).__name__}: {e}'
            self.in_play.clear()

    def _handle(self, i, payload):
        if self.state == 'login':
            if i == 0:
                self.reason = ('login disconnect', payload)
            elif i == 3:
                self.threshold = read_varint(payload)[0]
            elif i == 2:
                self.send(LOGIN_ACK)
                self.state = 'config'
        elif self.state == 'config':
            if i == CFG_DISCONNECT:
                self.reason = ('config disconnect', payload)
            elif i == CFG_KEEP_ALIVE:
                self.send(CFG_KEEP_ALIVE, payload)
            elif i == CFG_PING:
                self.send(CFG_PING, payload)
            elif i == CFG_KNOWN_PACKS:
                self.send(7, varint(0))  # no known packs: the server sends every registry whole
            elif i == CFG_FINISH:
                self.send(CFG_FINISH)
                self.state = 'play'
        elif self.state == 'play':
            if i == PLAY_DISCONNECT:
                self.reason = ('play disconnect', payload)
            elif i == PLAY_KEEP_ALIVE_IN:
                self.send(PLAY_KEEP_ALIVE_OUT, payload)
            elif i == PLAY_LOGIN:
                self.in_play.set()
            elif i == PLAY_START_CONFIG:
                self.in_play.clear()
                self.send(PLAY_CONFIG_ACK)
                self.state = 'config'

    def wait_play(self, seconds=120):
        if not self.in_play.wait(seconds):
            raise Failed(f'{self.name} did not reach play (state {self.state}, reason {self.reason})')

    def command(self, text):
        self.send(PLAY_CHAT_COMMAND, string(text))

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


def status_motd(port):
    sock = socket.create_connection(('127.0.0.1', port), timeout=30)
    sock.sendall(varint(len(b := varint(0) + varint(PROTOCOL) + string('localhost') + struct.pack('>H', port) + varint(1))) + b)
    sock.sendall(b'\x01\x00')
    data = b''
    while True:
        chunk = sock.recv(65536)
        data += chunk
        try:
            length, i = read_varint(data)
            if len(data) >= i + length:
                break
        except IndexError:
            pass
        if not chunk:
            break
    sock.close()
    body = data[i:i + length]
    _, j = read_varint(body)
    text_len, k = read_varint(body, j)
    return json.loads(body[k:k + text_len])


def cli(bins, folder, *words):
    return subprocess.run([str(bins / f'horizon-server{EXE}'), '--dir', str(folder), *map(str, words)],
                          capture_output=True, text=True, env=lab.game_env(), timeout=120)


def wait_for(condition, what, seconds=60):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if condition():
            return
        time.sleep(0.5)
    raise Failed(f'timed out waiting for {what}')


def scenario_join(bins, folder, port, game_port):
    wait_for(lambda: 'proxy e2e' in json.dumps(status_motd(port)['description']),
             'the server list through the proxy to show the game\'s description')
    check(True, 'the server list through the proxy is the game\'s')
    player = Client(port, NAME)
    player.wait_play()
    check(player.state == 'play', f'{NAME} reached play through the proxy')
    out = cli(bins, folder, 'cmd', 'op', NAME)
    check(out.returncode == 0, f'the console ops {NAME} ({out.stdout!r} {out.stderr!r})')
    time.sleep(2)
    player.command('give @s minecraft:diamond 7')
    wait_for(lambda: '7' in cli(bins, folder, 'cmd', 'data', 'get', 'entity', NAME, 'Inventory').stdout,
             'the game to run the player\'s /give')
    check(True, f'{NAME}\'s command ran in the game and gave the items')
    cli(bins, folder, 'cmd', 'save-all')
    saved = folder / 'world' / 'players' / 'data' / f'{offline_uuid(NAME)}.dat'
    wait_for(saved.exists, f'the game to save {NAME} as {offline_uuid(NAME)}')
    check(True, f'the game knows {NAME} by the UUID the login gave: {saved.name}')
    direct = Client(game_port, 'Direct')
    wait_for(direct.closed.is_set, 'a direct login to be refused')
    check(not direct.in_play.is_set(), 'a client that skips the proxy is refused by the game')
    return player


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-proxy-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    work = Path(tempfile.mkdtemp(prefix='hz-server-proxy-'))
    folder = work / 'server'
    folder.mkdir()
    port, game_port = lab.free_port(), lab.free_port()
    (folder / 'eula.txt').write_text('eula=true\n')
    (folder / 'server.toml').write_text(f'''[server]
port = {port}
motd = "proxy e2e"
online_mode = false
whitelist = false
view_distance = 3
simulation_distance = 3

[properties]
level-type = "minecraft:flat"
generate-structures = false
spawn-protection = 0

[engine]
path = {json.dumps(str(bins / f'horizon-client{EXE}'))}
resources = {json.dumps(str(lab.RESOURCES.resolve()))}
restart_on_crash = false

[backups]
every = "0"

[panel]
bind = ""

[proxy]
enabled = true
game_port = {game_port}
''')
    server = Server([bins / f'horizon-server{EXE}', '--dir', folder, 'run'], folder, out / 'server.log')
    player = None
    try:
        server.wait_for(DONE, 300)
        player = scenario_join(bins, folder, port, game_port)
        cli(bins, folder, 'stop')
        server.process.wait(timeout=120)
    except (Failed, RuntimeError, OSError, subprocess.TimeoutExpired) as e:
        sys.exit(f'server_proxy_e2e failed (logs in {out}): {e}')
    finally:
        if player:
            player.close()
        server.stop()
        shutil.rmtree(work, ignore_errors=True)
    print('server_proxy_e2e: a player joined through the proxy, chatted a command the game ran for them, and '
          'the game knew them by their real UUID')


if __name__ == '__main__':
    main()
