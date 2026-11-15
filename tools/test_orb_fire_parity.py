#!/usr/bin/env python3
"""Real Game against vanilla: an experience orb's saved Fire and its once-per-20-ticks burn."""
import argparse
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    command = [sys.executable, str(root / 'tools/test_command_exec_parity.py'),
               '--orb-fire-fixture', '--work-dir', str(root / '.local' / 'orb-fire-parity')]
    if args.binary:
        command += ['--binary', str(args.binary.resolve())]
    if args.prepare:
        command.append('--prepare')
    return subprocess.run(command, cwd=root).returncode


if __name__ == '__main__':
    sys.exit(main())
