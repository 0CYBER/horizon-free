#!/usr/bin/env python3
"""Dedicated native slot regression, called by the ignored launcher source-server test."""
import argparse
import os
from pathlib import Path
import select
import subprocess
import tempfile
import time

import lab


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--resources', required=True, type=Path)
    parser.add_argument('--scratch', required=True, type=Path)
    parser.add_argument('--functions', action='store_true')
    parser.add_argument('--calls', action='store_true')
    parser.add_argument('--wrapping', action='store_true')
    args = parser.parse_args()
    scratch = Path(tempfile.mkdtemp(prefix='slot-order-', dir=args.scratch))
    env = os.environ.copy()
    for key in ['HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME',
                'HORIZON_LAUNCHER_TRASH', 'HORIZON_LAUNCHER_CLIPBOARD', 'HORIZON_LAUNCHER_DESKTOP']:
        env[key] = tempfile.mkdtemp(prefix=key.lower(), dir=scratch)
    env = lab.game_env(env)
    sysroot = subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip()
    env['LD_LIBRARY_PATH'] = f'{args.binary.parent}/deps:{sysroot}/lib/rustlib/x86_64-unknown-linux-gnu/lib'
    port = lab.free_port()
    (scratch / 'eula.txt').write_text('eula=true\n')
    (scratch / 'server.properties').write_text(
        f'online-mode=false\nserver-ip=127.0.0.1\nserver-port={port}\nlevel-seed=12345\n'
        'view-distance=2\nsimulation-distance=2\npause-when-empty-seconds=-1\ngenerate-structures=false\n')
    command = [str(args.binary), '--resources', str(args.resources), '--game-dir', str(scratch),
               '--dedicated', '--nogui', '--world', 'Proof']
    process = subprocess.Popen(command, cwd=scratch, env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = bytearray()
    ready = stopped = False
    try:
        start = time.monotonic()
        while time.monotonic() - start < 240:
            if select.select([process.stdout], [], [], 1)[0]:
                chunk = os.read(process.stdout.fileno(), 65536)
                if not chunk:
                    break
                output.extend(chunk)
                text = output.decode(errors='replace')
                if not ready and 'Done (' in text:
                    ready = True
                    process.stdin.write(b'tick rate 1\n')
                    process.stdin.flush()
                if ready and not stopped and 'SLOT ORDER LOW APPLIED ticks_until_autosave=319' in text:
                    stopped = True
                    process.stdin.write(b'stop\n')
                    process.stdin.flush()
            if process.poll() is not None:
                output.extend(process.stdout.read())
                break
        if process.poll() is None:
            process.terminate()
        code = process.wait(timeout=20)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        (scratch / 'transcript.log').write_bytes(output)
    text = output.decode(errors='replace')
    assert ready and stopped and code == 0, text[-4000:]
    expected = ['SLOT ORDER API HEAD', 'SLOT ORDER HEAD', 'SLOT ORDER MODIFY_VALUE: 300 -> 307',
                'SLOT ORDER API RETURN: 307 -> 308', 'SLOT ORDER WRAP_CALL: 308 -> 319',
                'SLOT ORDER HIGH ticks_until_autosave=319', 'SLOT ORDER LOW APPLIED ticks_until_autosave=319']
    if args.functions:
        expected = ['SLOT ORDER HEAD', 'SLOT ORDER SOURCE RETURN: 300 -> 319',
                    'SLOT ORDER HIGH ticks_until_autosave=319', 'SLOT ORDER LOW APPLIED ticks_until_autosave=319']
    if args.calls:
        expected = ['SLOT ORDER HEAD', 'SLOT ORDER CALL BEFORE rate=1',
                    'SLOT ORDER CALL AFTER value=300', 'SLOT ORDER CALL VALUE: 300 -> 319',
                    'SLOT ORDER HIGH ticks_until_autosave=319', 'SLOT ORDER LOW APPLIED ticks_until_autosave=319']
    if args.wrapping:
        expected.insert(expected.index('SLOT ORDER CALL VALUE: 300 -> 319') + 1,
                        'SLOT ORDER CALL WRAP: 300 -> 319')
    positions = [text.index(marker) for marker in expected]
    assert positions == sorted(positions), text[-4000:]
    print('PASS:', '; '.join(expected), '; exit 0', scratch)


if __name__ == '__main__':
    main()
