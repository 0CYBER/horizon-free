#!/usr/bin/env python3
"""Run the vanilla structure-start oracle for a seed and chunk rectangle.

usage: structure_start_oracle.py OUT SEED X0 Z0 X1 Z1
"""
import os
import subprocess
from pathlib import Path
import sys
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def main():
    out, seed, x0, z0, x1, z1 = sys.argv[1:7]
    lock, jars, libraries = base.verify_artifacts()
    source = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStructureStartOracle.java'
    build = Path(os.environ['HORIZON_ORACLE_BUILD']) if os.environ.get('HORIZON_ORACLE_BUILD') else lab.LOCAL/'structure-start-oracle'
    classes = build/'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    compiled = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-cp', dependencies,
                               '-d', str(classes), str(source)], capture_output=True, text=True)
    if compiled.returncode:
        raise SystemExit('javac failed:\n' + compiled.stderr[-3000:])
    result = subprocess.run(['java', '-Xmx2G', '-cp', str(classes)+os.pathsep+dependencies,
                             'dev.horizonlab.parity.VanillaStructureStartOracle', out, seed, x0, z0, x1, z1],
                            capture_output=True, text=True)
    if result.returncode:
        lines = [line for line in (result.stdout + result.stderr).splitlines() if 'Exception' in line or 'Caused' in line]
        raise SystemExit('oracle failed: ' + ' | '.join(lines[:5]))


if __name__ == '__main__':
    main()
