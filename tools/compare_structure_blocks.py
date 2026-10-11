#!/usr/bin/env python3
"""Compare structure placement (terrain + structure pieces, no placed features)
between vanilla and Rust for target chunks.

usage: compare_structure_blocks.py RESOURCES SEED DIMENSION CX,CZ...
Runs VanillaStructurePlacementOracle and the structure_blocks example and
reports, per chunk, the blocks and block entities that differ.
"""
import json
import os
import subprocess
import sys
from pathlib import Path
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def build_dir():
    return Path(os.environ['HORIZON_ORACLE_BUILD']) if os.environ.get('HORIZON_ORACLE_BUILD') else lab.LOCAL/'structure-placement-oracle'


def run_oracle(out, args):
    lock, jars, libraries = base.verify_artifacts()
    source = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStructurePlacementOracle.java'
    classes = build_dir()/'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    compiled = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-cp', dependencies,
                               '-d', str(classes), str(source)], capture_output=True, text=True)
    if compiled.returncode:
        raise SystemExit('javac failed:\n' + compiled.stderr[-3000:])
    result = subprocess.run(['java', '-Xmx4G', '-cp', str(classes)+os.pathsep+dependencies,
                             'dev.horizonlab.parity.VanillaStructurePlacementOracle', str(out), *args],
                            capture_output=True, text=True)
    if result.returncode:
        lines = [line for line in (result.stdout + result.stderr).splitlines() if 'Exception' in line or 'Error' in line or '\tat ' in line]
        raise SystemExit('oracle failed: ' + ' | '.join(lines[:8]))


def parse(lines):
    chunks, current = {}, None
    for line in lines:
        t = line.split()
        if not t:
            continue
        if t[0] == 'chunk':
            current = (int(t[1]), int(t[2]))
            chunks[current] = {'rows': {}, 'be': []}
        elif t[0] == 'y' and current:
            chunks[current]['rows'][int(t[1])] = t[2:]
        elif t[0] == 'be' and current:
            # Position, loot table and seed; the block entity type follows
            # from the (separately compared) block state.
            chunks[current]['be'].append(' '.join(t[1:4] + t[5:]))
    return chunks


def state_names(resources):
    names = {}
    report = lab.LOCAL/'datagen/out/reports/blocks.json'
    if report.exists():
        for block, info in json.loads(report.read_text()).items():
            for state in info['states']:
                props = ','.join(f'{k}={v}' for k, v in sorted(state.get('properties', {}).items()))
                names[str(state['id'])] = block + (f'[{props}]' if props else '')
    return names


def main():
    resources, seed, dimension, targets = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
    out = build_dir()/'vanilla.txt'
    out.parent.mkdir(parents=True, exist_ok=True)
    run_oracle(out, [seed, dimension, *targets])
    vanilla = parse(out.read_text().splitlines())
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-q', '-p', 'horizon-world', '--example', 'structure_blocks'],
                   check=True, cwd=lab.ROOT)
    rust_out = subprocess.run([str(lab.cargo_target()/'fast/examples/structure_blocks'), resources, seed, dimension, *targets],
                              check=True, capture_output=True, text=True).stdout
    (build_dir()/'rust.txt').write_text(rust_out)
    rust = parse(rust_out.splitlines())
    names = state_names(resources)
    failed = 0
    for key in sorted(vanilla):
        v, r = vanilla[key], rust.get(key)
        if r is None:
            print(key, 'missing from Rust')
            failed += 1
            continue
        diffs = []
        for y, row in v['rows'].items():
            other = r['rows'].get(y, [])
            for i, (a, b) in enumerate(zip(row, other)):
                if a != b:
                    diffs.append((key[0] * 16 + i % 16, y, key[1] * 16 + i // 16, a, b))
        total = sum(len(row) for row in v['rows'].values())
        be_v, be_r = set(v['be']), set(r['be'])
        status = 'match' if not diffs and be_v == be_r else 'DIFF'
        print(f'chunk {key}: {status}; {total - len(diffs)} of {total} blocks equal; block entities {len(be_v & be_r)} of {len(be_v)} equal')
        for x, y, z, a, b in diffs[:12]:
            print(f'  {x} {y} {z} vanilla {names.get(a, a)} rust {names.get(b, b)}')
        for e in sorted(be_v - be_r)[:6]:
            print('  only vanilla be', e)
        for e in sorted(be_r - be_v)[:6]:
            print('  only rust be', e)
        failed += status != 'match'
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
