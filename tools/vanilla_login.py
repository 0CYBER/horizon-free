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
import socket
import struct
import sys
import time
import zlib

try:
    from . import lab
except ImportError:
    import lab


def string(s):
    data = s.encode()
    return lab.varint(len(data)) + data


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
        body = lab.varint(packet_id) + payload
        if self.threshold >= 0:
            if len(body) >= self.threshold:
                body = lab.varint(len(body)) + zlib.compress(body)
            else:
                body = lab.varint(0) + body
        self.sock.sendall(lab.varint(len(body)) + body)

    def receive(self):
        length = self.read_varint()
        body = self.read_exact(length)
        if self.threshold >= 0:
            pos, size = lab.parse_varint(body, 0)
            body = zlib.decompress(body[pos:]) if size else body[pos:]
        pos, packet_id = lab.parse_varint(body, 0)
        return packet_id, body[pos:]


def join(host, port, name, seconds):
    c = Connection(host, port)
    c.send(0, lab.varint(lab.version_info()['protocol_version']) + string(host) + struct.pack('>H', port) + lab.varint(2))
    c.send(0, string(name) + lab.offline_uuid(name).bytes)
    # Login: compression, then login finished.
    while True:
        pid, data = c.receive()
        if pid == 3:
            c.threshold = lab.parse_varint(data, 0)[1]
        elif pid == 2:
            break
        elif pid == 0:
            raise RuntimeError(f'login refused: {data!r}')
    c.send(3)  # login acknowledged -> configuration
    # Configuration packet ids below are from crates/horizon-protocol/src/ids.rs.
    while True:
        pid, data = c.receive()
        if pid == 15:  # select known packs: claim none, receive full registries
            c.send(7, lab.varint(0))
        elif pid == 0:  # cookie request: this client has no stored cookie payload
            c.send(1, data + b'\x00')  # preserve the key, then encode an absent payload
        elif pid == 4:  # keep alive
            c.send(4, data)
        elif pid == 5:  # ping
            c.send(5, data)
        elif pid == 3:  # finish configuration
            c.send(3)
            break
        elif pid == 2:
            raise RuntimeError(f'disconnected during configuration: {data!r}')
        elif pid == 20:  # code of conduct: the server waits for packet 9 acceptance
            raise RuntimeError(
                'unhandled configuration packet 20 (code_of_conduct): cannot accept server terms')
        elif pid in (8, 9):  # resource pack pop/push
            # Answering a push needs a pack UUID and an acceptance code; this
            # bare client cannot make one up, so fail instead of hanging.
            name = 'resource_pack_pop' if pid == 8 else 'resource_pack_push'
            raise RuntimeError(f'unhandled configuration packet {pid} ({name}): cannot answer it')
        elif pid == 1:  # custom payload: brand needs no answer; others might
            try:
                pos, size = lab.parse_varint(data, 0)
                channel = data[pos:pos + size].decode('utf-8', 'replace')
            except IndexError:
                channel = ''
            if channel != 'minecraft:brand':
                raise RuntimeError(
                    f'unhandled configuration packet 1 (custom_payload) on channel {channel!r}')
        # Registry data, tags, feature flags, server links, etc. need no answer.
    deadline = time.time() + seconds
    c.sock.settimeout(1.0)
    while time.time() < deadline:
        try:
            pid, data = c.receive()
        except socket.timeout:
            continue
        except EOFError:
            break  # the server stopped
        if pid == 45:  # play keep alive: echo the payload so we are not kicked
            c.send(28, data)
        # Every other play packet is irrelevant to this bare client.
    c.sock.close()


def main():
    p = argparse.ArgumentParser()
    p.add_argument('name')
    p.add_argument('--host', default='127.0.0.1')
    p.add_argument('--port', type=int, default=25565)
    p.add_argument('--seconds', type=float, default=5,
                   help='seconds to stay connected, at most 25 (default: 5)')
    p.add_argument('--uuid', action='store_true')
    args = p.parse_args()
    if args.uuid:
        print(lab.offline_uuid(args.name))
        return 0
    if args.seconds > 25:
        p.error('--seconds must be at most 25 (got %s)' % args.seconds)
    join(args.host, args.port, args.name, args.seconds)
    return 0


if __name__ == '__main__':
    sys.exit(main())
