#!/usr/bin/env python3
"""Upgrade a world saved by an older Minecraft version to the 26.3 DataVersion.

Horizon reads chunks only at the current DataVersion and has not ported the
DataFixerUpper chain. This runs vanilla's own upgrader (`server.jar --forceUpgrade
--eraseCache`) on a copy of the world, so every fix vanilla applies (chunk, entity, POI,
level.dat, player and data/*.dat files) is applied exactly. The original is kept.

  upgrade_world.py SOURCE_WORLD DEST_WORLD
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parent.parent
SERVER = lab.GAME / 'jars' / 'server.jar'


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    source, dest = Path(sys.argv[1]), Path(sys.argv[2])
    if not (source / 'level.dat').is_file():
        sys.exit(f'{source} is not a world (no level.dat)')
    if dest.exists():
        sys.exit(f'{dest} exists already')
    with tempfile.TemporaryDirectory(prefix='hzupgrade') as scratch:
        scratch = Path(scratch)
        (scratch / 'eula.txt').write_text('eula=true\n')
        (scratch / 'server.properties').write_text(
            f'level-name=world\nonline-mode=false\nserver-port={lab.free_port()}\n')
        shutil.copytree(source, scratch / 'world')
        # The server upgrades at startup, then keeps running as a normal server: stop it
        # once it reports "Done".
        server = subprocess.Popen(
            ['java', '-jar', str(SERVER), '--nogui', '--forceUpgrade', '--eraseCache'],
            cwd=scratch, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        for line in server.stdout:
            sys.stdout.write(line)
            if 'Done (' in line:
                server.stdin.write('stop\n')
                server.stdin.flush()
        if server.wait():
            sys.exit('vanilla upgrade failed')
        shutil.move(str(scratch / 'world'), str(dest))
    print(f'upgraded world written to {dest}')


if __name__ == '__main__':
    main()
