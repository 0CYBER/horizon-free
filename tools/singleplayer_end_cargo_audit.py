#!/usr/bin/env python3
"""Check #1518: mobs and dropped items go through an End gateway.

Build horizon-client and horizon-ctl with the fast profile first, then run:
  python3 tools/singleplayer_end_cargo_audit.py --case gateway
Runs both the default server-thread and the in-frame singleplayer, in scratch
homes: a cow and a seven-diamond stack stand in a gateway in the End, and
both are expected to leave it and land at the gateway's exit, the stack with
its seven diamonds.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

from agent_control_e2e import Cli
import lab


# The gateway beside the End's spawn platform, where the player waits.
GATEWAY = (104, 50, 0)
# The diamonds the dropped stack carries.
DIAMONDS = 7


def data_get(cli, selector, path):
    """The chat line of a `/data get entity`."""
    since = cli('events')['last']
    cli('command', command=f'data get entity {selector} {path}')
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        reply = cli('events', since=since)
        for event in reply['events']:
            text = event.get('text', '')
            if event.get('event') == 'chat' and path in text:
                return text
        time.sleep(0.1)
    raise AssertionError(f'no answer for /data get entity {selector} {path}')


def numbers(text):
    """The numbers of a `/data get` answer: "Pos: [104.5, 50.0, 0.5]"."""
    return [float(v) for v in re.findall(r'-?\d+(?:\.\d+)?', text.split(':', 1)[-1])]


def run(bin_dir, resources, in_frame):
    with tempfile.TemporaryDirectory(prefix='hz-end-gateway-') as scratch:
        work = Path(scratch)
        env = lab.game_env({k: v for k, v in os.environ.items() if k not in (
            'HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')})
        home = Path(env['HORIZON_HOME'])
        socket = work / 'game.sock'
        cli = Cli(bin_dir / 'horizon-ctl', socket, home, env)
        args = [str(bin_dir / 'horizon-client'), '--resources', str(resources),
                '--world', 'cargo', '--creative', '--seed', '4242', '--offscreen',
                '--render-distance', '4', '--control', '--control-socket', str(socket)]
        if in_frame:
            args.append('--in-frame')
        with (work / 'game.log').open('w') as log:
            process = subprocess.Popen(args, cwd=home, env=env, stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 120
                while not socket.exists():
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError('game did not open its control socket')
                    time.sleep(0.1)
                cli('wait_ready', seconds=240)
                # The End, on its spawn platform.
                cli('command', command='execute in minecraft:the_end run tp @s 100 50 0')
                for _ in range(60):
                    cli('wait', ticks=20)
                    if cli('state').get('dimension') == 'minecraft:the_end':
                        break
                else:
                    raise AssertionError('did not enter the End')
                cli('wait_ready', seconds=240)
                # A gateway beside the player, with a cow and a stack in it.
                x, y, z = GATEWAY
                cli('command', command=(
                    f'execute in minecraft:the_end run setblock {x} {y} {z} minecraft:end_gateway'))
                cli('command', command=(
                    f'execute in minecraft:the_end run summon minecraft:cow {x}.5 {y}.5 {z}.5 {{NoAI:1b}}'))
                cli('command', command=(
                    f'execute in minecraft:the_end run summon minecraft:item {x}.5 {y}.5 {z}.5 '
                    f'{{Item:{{id:"minecraft:diamond",count:{DIAMONDS}}}}}}'))
                # The gateway takes the cow, and the stack 40 ticks later.
                cli('wait', ticks=200)
                cow = numbers(data_get(cli, '@e[type=minecraft:cow,limit=1]', 'Pos'))
                item = numbers(data_get(cli, '@e[type=minecraft:item,limit=1]', 'Pos'))
                count = numbers(data_get(cli, '@e[type=minecraft:item,limit=1]', 'Item.count'))
                print(f'cow at {cow}, item at {item}, count {count}')
                for name, at in (('cow', cow), ('item', item)):
                    assert len(at) == 3, f'{name}: no position in {at}'
                    assert abs(at[0] - x) > 64 or abs(at[2] - z) > 64, \
                        f'{name} stayed in the gateway at {at}'
                assert count and int(count[0]) == DIAMONDS, f'the stack lost diamonds: {count}'
                cli('quit')
                assert process.wait(timeout=120) == 0
            except Exception:
                log.flush()
                print((work / 'game.log').read_text(errors='replace')[-6000:])
                raise
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=30)
        print(f'PASS end gateway: {"in-frame" if in_frame else "server-thread"}, '
              f'the cow and the stack left the gateway with their data')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=['gateway'], default='gateway')
    parser.add_argument('--bin-dir', type=Path,
                        default=Path(__file__).resolve().parents[1] / 'target/fast')
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    args = parser.parse_args()
    for in_frame in (False, True):
        run(args.bin_dir.resolve(), args.resources.resolve(), in_frame)


if __name__ == '__main__':
    main()
