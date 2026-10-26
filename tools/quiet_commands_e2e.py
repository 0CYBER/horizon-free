#!/usr/bin/env python3
"""Mods' quiet commands on Horizon's dedicated server, end to end (horizon_protocol::quiet).

Horizon's dedicated server (a flat world) and Horizon's headless client (`HORIZON_NET_REPORT`, the net
harness), which joins, then sends a batch of quiet commands as a mod would (`HORIZON_NET_QUIET`):

- as a player who isn't an operator: `setblock` is refused (the block stays air), as it would be typed;
- as an operator: `setblock` runs, a `fill` that changes nothing fails, and `say` is refused (a chat
  message needs the player's signature);
- either way the player's chat gets nothing of them, and the server logs one line per batch: the player,
  the mod, how many ran and how many failed.

    python3 tools/quiet_commands_e2e.py --bin-dir target/fast

The client runs with fresh HORIZON_HOME, launcher and XDG folders and without audio; the logs go to --out.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from integration_env import run_game  # noqa: E402
from test_dedicated_server_parity import Server  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
PLAYER = 'Tester'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'spawn-protection': '0',
    'view-distance': '3',
    'simulation-distance': '3',
    'pause-when-empty-seconds': '-1',
}
BLOCK = '2 -55 2'
# What a player would see of these commands, were they typed.
FEEDBACK = ['Changed the block', 'No blocks were filled', 'Unknown or incomplete command', 'hi from a mod',
            'Could not set the block']


def join(binary, port, out, label, lines):
    """The headless client joins, sends `lines` as one quiet batch and leaves: its report."""
    home = Path(tempfile.mkdtemp(prefix=f'quiet-{label}-'))
    report = out / f'{label}-report.json'
    # run_game gives the game fresh home, launcher and XDG folders.
    env = dict(HORIZON_NET_REPORT=str(report), HORIZON_NET_QUIET=';'.join(lines))
    with (out / f'{label}-client.log').open('w') as log:
        run_game([str(binary), '--resources', str(lab.RESOURCES), '--render-distance', '2',
                  '--server', f'127.0.0.1:{port}', '--name', PLAYER],
                 cwd=home, env=dict(os.environ, **env), stdout=log, stderr=subprocess.STDOUT, timeout=300,
                 check=True)
    shutil.rmtree(home, ignore_errors=True)
    return json.loads(report.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='a prebuilt horizon-client (default: build it with the fast profile)')
    parser.add_argument('--out', default=str(ROOT / '.local/quiet-commands-e2e'), help='logs go here')
    args = parser.parse_args()
    out = Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    if args.bin_dir:
        binary = Path(args.bin_dir) / 'horizon-client'
    else:
        subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'],
                       cwd=ROOT, check=True)
        binary = Path(lab.cargo_target()) / 'fast' / 'horizon-client'
    port = lab.free_port()
    directory = out / 'server'
    directory.mkdir()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    server = Server([binary, '--resources', lab.RESOURCES, '--dedicated', '--nogui'], directory, 'server')
    failures = []

    def check(what, ok, detail=''):
        print(('PASS ' if ok else 'FAIL ') + what + ('' if ok else f': {detail}'))
        if not ok:
            failures.append(what)

    def test_block(want):
        """The console's `execute if block`: 'Test passed' when the block is `want`."""
        before = len(server.lines)
        server.console(f'execute if block {BLOCK} {want}')
        deadline = time.time() + 30
        while time.time() < deadline:
            answer = [l for l in server.lines[before:] if 'Test passed' in l or 'Test failed' in l]
            if answer:
                return 'Test passed' in answer[0]
            time.sleep(0.2)
        raise AssertionError('the console never answered execute if block')

    try:
        server.wait_for(r'Done \(')
        # Not an operator: the setblock is refused, as typed.
        got = join(binary, port, out, 'not-op', [f'setblock {BLOCK} minecraft:gold_block'])
        server.wait_for(rf'{PLAYER} ran 1 quiet command\(s\) from mod net_harness: 1 failed', timeout=30)
        check('not an operator: connected throughout', got['error'] is None, got['error'])
        check("not an operator: a quiet setblock is refused", not test_block('minecraft:gold_block'))
        heard = [c for c in got['chat'] if any(f in c for f in FEEDBACK)]
        check('not an operator: nothing in the chat', not heard, heard)
        server.wait_for(rf'{PLAYER} left the game', timeout=60)
        # An operator: it runs, quietly.
        server.console(f'op {PLAYER}')
        server.wait_for(rf'Made {PLAYER} a server operator', timeout=30)
        got = join(binary, port, out, 'op', [f'setblock {BLOCK} minecraft:gold_block',
                                             f'fill {BLOCK} {BLOCK} minecraft:gold_block', 'say hi from a mod'])
        server.wait_for(rf'{PLAYER} ran 3 quiet command\(s\) from mod net_harness: 2 failed', timeout=30)
        check('operator: connected throughout', got['error'] is None, got['error'])
        check('operator: the quiet setblock ran', test_block('minecraft:gold_block'))
        heard = [c for c in got['chat'] if any(f in c for f in FEEDBACK)]
        check('operator: nothing in the chat (no feedback, no failure, no say)', not heard, heard)
        said = [l for l in server.lines if 'hi from a mod' in l]
        check('operator: say is refused', not said, said)
    finally:
        server.stop()
    print(f'logs: {out}')
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
