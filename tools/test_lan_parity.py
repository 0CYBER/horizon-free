#!/usr/bin/env python3
"""LAN world discovery against the pinned client's LanServerPinger and
LanServerDetection: the announcement an opened world multicasts
(`[MOTD]...[/MOTD][AD]port[/AD]` in UTF-8), and how the multiplayer screen
parses what it hears (parseMotd, parseAddress, UTF-8 decoding of malformed
bytes) and keeps its list (LanServerList.addServer: one entry per
address, the first MOTD kept, never expired while the screen is open).
VanillaLanOracle and the `lan_probe` example must print the same lines."""
import random

try:
    from . import oracle_harness as oh
except ImportError:
    import oracle_harness as oh

NAME = 'lan-parity'


def make_input(build):
    rng = random.Random(4445)
    lines = []
    motds = ['Player - New World', 'Steve - §aGreen §lWorld', 'été 日本語 ✓', '', '[MOTD]', 'a[/MOTD]b', 'x' * 300,
             'tab\\there', '[AD]25565[/AD]']
    for motd in motds:
        for port in ['25565', '1024', '65535', '0', 'abc', '']:
            lines.append(f'ping\t{motd}\t{port}')
    fragments = [b'[MOTD]', b'[/MOTD]', b'[AD]', b'[/AD]', b'25565', b'1234', b'Steve - World', b'\xc3\xa9', b'\xe6\x97',
                 b'\xff', b'\xc0\x80', b'\xed\xa0\x80', b'\xf0\x9f\x98\x80', b'\xf0\x9f', b'abc', b' ', b'\x00', b'-1']
    ips = ['192.168.1.10', '10.0.0.2', '127.0.0.1']
    good = lambda motd, port: f'[MOTD]{motd}[/MOTD][AD]{port}[/AD]'.encode()
    for i in range(400):
        if i % 50 == 0:
            lines.append('reset')
        if rng.random() < 0.4:
            data = good(rng.choice(['A - W', 'B - W', 'é', '']), rng.choice(['25565', '25566', 'x', '']))
        else:
            data = b''.join(rng.choice(fragments) for _ in range(rng.randint(0, 9)))
        lines.append(f'heard\t{rng.choice(ips)}\t{data.hex()}')
    path = build / 'cases.txt'
    path.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    return path, len(lines)


if __name__ == '__main__':
    oh.differential(NAME, 'VanillaLanOracle', 'lan_probe', make_input,
                    'LanServerPinger.createPingString/parseMotd/parseAddress, LanServerDetection decoding and '
                    'LanServerList.addServer/takeDirtyServers',
                    sources=[oh.lab.ROOT / 'crates/horizon-client/src/lan/discovery.rs'])
