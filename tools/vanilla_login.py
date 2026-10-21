#!/usr/bin/env python3
"""Join an offline-mode vanilla 26.3 server as a bare protocol client, stay
connected for a while (answering keep-alives) and leave.

  vanilla_login.py [--host H] [--port P] [--seconds S] NAME
  vanilla_login.py --uuid NAME      print the offline UUID the server uses

The server loads and saves the player's file (players/data/<uuid>.dat), so
this checks that player data written by Horizon is accepted by vanilla:
its log reports the position it read, and console commands such as
`data get entity NAME` can inspect the loaded player meanwhile.
"""
import argparse
import hashlib
import socket
import struct
import sys
import time
import uuid
import zlib

try:
    from . import lab
except ImportError:
    import lab


def offline_uuid(name):
    digest = bytearray(hashlib.md5(('OfflinePlayer:' + name).encode()).digest())
    digest[6] = (digest[6] & 0x0F) | 0x30
    digest[8] = (digest[8] & 0x3F) | 0x80
    return uuid.UUID(bytes=bytes(digest))


def varint(v):
    out = b''
    v &= 0xFFFFFFFF
    while True:
        b = v & 0x7F
        v >>= 7
        out += bytes([b | (0x80 if v else 0)])
        if not v:
            return out


def string(s):
    data = s.encode()
    return varint(len(data)) + data


class Connection:
    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=30)
        self.threshold = -1
        self.buffer = b''

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
        body = varint(packet_id) + payload
        if self.threshold >= 0:
            if len(body) >= self.threshold:
                body = varint(len(body)) + zlib.compress(body)
            else:
                body = varint(0) + body
        self.sock.sendall(varint(len(body)) + body)

    def receive(self):
        length = self.read_varint()
        body = self.read_exact(length)
        if self.threshold >= 0:
            pos, size = parse_varint(body, 0)
            body = zlib.decompress(body[pos:]) if size else body[pos:]
        pos, packet_id = parse_varint(body, 0)
        return packet_id, body[pos:]


def parse_varint(data, pos):
    value = shift = 0
    while True:
        b = data[pos]
        pos += 1
        value |= (b & 0x7F) << shift
        shift += 7
        if not b & 0x80:
            return pos, value


def join(host, port, name, seconds):
    c = Connection(host, port)
    c.send(0, varint(lab.version_info()['protocol_version']) + string(host) + struct.pack('>H', port) + varint(2))
    c.send(0, string(name) + offline_uuid(name).bytes)
    # Login: compression, then login finished.
    while True:
        pid, data = c.receive()
        if pid == 3:
            c.threshold = parse_varint(data, 0)[1]
        elif pid == 2:
            break
        elif pid == 0:
            raise RuntimeError(f'login refused: {data!r}')
    c.send(3)  # login acknowledged -> configuration
    while True:
        pid, data = c.receive()
        if pid == 15:  # select known packs: claim none, receive full registries
            c.send(7, varint(0))
        elif pid == 4:  # keep alive
            c.send(4, data)
        elif pid == 5:  # ping
            c.send(5, data)
        elif pid == 3:  # finish configuration
            c.send(3)
            break
        elif pid == 2:
            raise RuntimeError(f'disconnected during configuration: {data!r}')
    deadline = time.time() + seconds
    c.sock.settimeout(1.0)
    while time.time() < deadline:
        try:
            c.receive()
        except socket.timeout:
            continue
        except EOFError:
            break  # the server stopped
        # Play packets are not decoded: the server's first keep-alive comes
        # after 15 seconds and it waits 15 more for the answer, so a session
        # under 30 seconds needs none.
    c.sock.close()


def main():
    p = argparse.ArgumentParser()
    p.add_argument('name')
    p.add_argument('--host', default='127.0.0.1')
    p.add_argument('--port', type=int, default=25565)
    p.add_argument('--seconds', type=float, default=5, help='at most 25')
    p.add_argument('--uuid', action='store_true')
    args = p.parse_args()
    if args.uuid:
        print(offline_uuid(args.name))
        return 0
    join(args.host, args.port, args.name, args.seconds)
    return 0


if __name__ == '__main__':
    sys.exit(main())
