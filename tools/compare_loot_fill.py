#!/usr/bin/env python3
"""Compare Rust container loot (examples/loot_fill) with vanilla LootTable.fill.

usage: compare_loot_fill.py RESOURCES TABLES SEED...
TABLES is a comma-separated list or `chests` for every chest table. Builds the
loot_fill example, runs VanillaLootFillOracle and diffs slot contents
(enchantments compared as sets). Tables vanilla cannot fill without a level
(exploration maps) are reported and skipped.
"""
import collections
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
    return Path(os.environ['HORIZON_ORACLE_BUILD']) if os.environ.get('HORIZON_ORACLE_BUILD') else lab.LOCAL/'loot-fill-oracle'


def run_oracle(out, tables, seeds):
    lock, jars, libraries = base.verify_artifacts()
    source = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLootFillOracle.java'
    build = build_dir()
    classes = build/'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    compiled = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-cp', dependencies,
                               '-d', str(classes), str(source)], capture_output=True, text=True)
    if compiled.returncode:
        raise SystemExit('javac failed:\n' + compiled.stderr[-3000:])
    result = subprocess.run(['java', '-Xmx2G', '-cp', str(classes)+os.pathsep+dependencies,
                             'dev.horizonlab.parity.VanillaLootFillOracle', str(out), tables, *seeds],
                            capture_output=True, text=True)
    if result.returncode:
        lines = [line for line in (result.stdout + result.stderr).splitlines() if 'Exception' in line or 'Caused' in line]
        raise SystemExit('oracle failed: ' + ' | '.join(lines[:5]))


def parse(lines):
    fills, errors = collections.defaultdict(list), set()
    for line in lines:
        t = line.split()
        if not t:
            continue
        if t[0] == 'table':
            fills[(t[1], t[2])]
        elif t[0] == 'error':
            errors.add((t[1], t[2]))
        elif t[0] == 'fill':
            fills[(t[1], t[2])].append((int(t[3]), t[4], t[5], t[6], tuple(sorted(t[7:]))))
    return fills, errors


def main():
    resources, tables, seeds = sys.argv[1], sys.argv[2], sys.argv[3:]
    out = build_dir()/'vanilla.txt'
    out.parent.mkdir(parents=True, exist_ok=True)
    run_oracle(out, tables, seeds)
    vanilla, errors = parse(out.read_text().splitlines())
    names = sorted({k[0] for k in vanilla} | {k[0] for k in errors})
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-q', '-p', 'horizon-world', '--example', 'loot_fill'],
                   check=True, cwd=lab.ROOT)
    rust_out = subprocess.run([str(lab.cargo_target()/'fast/examples/loot_fill'), resources, ','.join(names), *seeds],
                              check=True, capture_output=True, text=True).stdout
    rust, _ = parse(rust_out.splitlines())
    matched = differ = 0
    for key in sorted(vanilla):
        if key in errors:
            continue
        if vanilla[key] == rust.get(key):
            matched += 1
            continue
        differ += 1
        if differ <= 20:
            print('DIFF', key, '\n  vanilla', vanilla[key], '\n  rust   ', rust.get(key))
    tables = collections.Counter(k[0] for k in vanilla if k not in errors and vanilla[k] != rust.get(k))
    if tables:
        print('differing tables:', dict(tables))
    skipped = sorted({k[0] for k in errors})
    print(f'{matched} of {matched + differ} fills match; skipped (needs a level): {skipped}')
    return 1 if differ else 0


if __name__ == '__main__':
    sys.exit(main())
