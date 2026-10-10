#!/usr/bin/env python3
"""Compare Horizon's resident PalettedContainer with the client's own.

Scripted edits run through vanilla's PalettedContainer (block-state and biome
strategies: single-value, linear, hash-map and global palettes, resizing, network
write and read, pack and unpack) and through horizon_core::paletted. Both sides
print the set results, the serialized container after every few edits, the
container read back from the wire, the saved form (palette, bit width, storage)
and the container loaded from it, as hashes.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

BUILD = lab.ROOT / 'target/paletted-container-parity'
LIMITS = {'blocks': (4096, 35723), 'biomes': (64, 67)}


def corpus():
    rng = random.Random(0x50414C45545445)
    counts = [1, 2, 3, 4, 5, 8, 9, 15, 16, 17, 31, 32, 33, 64, 100, 128, 129, 200, 256, 257, 300, 700, 2000]
    for kind, (entries, ids) in LIMITS.items():
        for count in counts:
            if kind == 'biomes' and count > 70:
                continue
            for trial in range(6):
                pool = rng.sample(range(ids), min(count, ids))
                initial = rng.choice(pool) if trial % 2 else rng.randrange(ids)
                ops = []
                for _ in range(rng.choice([entries // 4, entries, entries * 3])):
                    ops.append((rng.randrange(entries), rng.choice(pool)))
                if trial == 5:
                    # Every position written once, in order, then half of them reset.
                    ops = [(i, pool[i % len(pool)]) for i in range(entries)] + [(i, initial) for i in range(0, entries, 2)]
                yield f'{kind}_{count}_{trial}', kind, initial, rng.choice([1, 7, 37, 211]), ops
        # A few edits at a time: the palette grows one value at a time from a single value.
        for trial in range(20):
            ops = [(rng.randrange(entries), rng.randrange(ids)) for _ in range(rng.randrange(1, 40))]
            yield f'{kind}_sparse_{trial}', kind, rng.randrange(ids), 1, ops
        # Overwriting with the value already there, and filling a section with one value.
        yield f'{kind}_fill', kind, 3, 500, [(i, 9) for i in range(entries)] + [(i, 3) for i in range(entries)]
        yield f'{kind}_same', kind, 3, 1, [(i, 3) for i in range(0, entries, 5)]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    BUILD.mkdir(parents=True, exist_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPalettedContainerOracle.java'
    sources = [Path(__file__), oracle, lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-core/Cargo.toml', lab.ROOT / 'crates/horizon-core/examples/paletted_fixture.rs',
               *sorted((lab.ROOT / 'crates/horizon-core/src').rglob('*.rs'))]
    tested = base.source_hashes(sources)
    scenarios = list(corpus())
    inputs = BUILD / 'inputs.txt'
    inputs.write_text(''.join(f'{name} {kind} {initial} {every} ' + ' '.join(f'{i} {v}' for i, v in ops) + '\n'
                              for name, kind, initial, every, ops in scenarios))

    def run(command, name):
        with (BUILD / (name + '.log')).open('w') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
    classes = BUILD / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies, '-d', classes, oracle], 'compile-java')
    run(['cargo', 'build', '--locked', '--profile', 'fast', '-j', os.environ.get('CARGO_BUILD_JOBS', '3'), '-p', 'horizon-core', '--example', 'paletted_fixture'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'fast/examples/paletted_fixture'
    run([binary, inputs, BUILD / 'rust.txt'], 'rust')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaPalettedContainerOracle', inputs, BUILD / 'client.txt'], 'client')
    client = (BUILD / 'client.txt').read_text().splitlines()
    rust = (BUILD / 'rust.txt').read_text().splitlines()
    assert len(client) == len(rust) == len(scenarios), (len(client), len(rust), len(scenarios))
    bad = [(a, b) for a, b in zip(client, rust) if a != b]
    base.ensure_sources_unchanged(sources, tested)
    for a, b in bad[:5]:
        print('client', a[:600])
        print('rust  ', b[:600])
    if bad:
        raise SystemExit(f'{len(bad)} of {len(scenarios)} scenarios differ')
    edits = sum(len(s[4]) for s in scenarios)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'scenarios': len(scenarios), 'edits': edits,
              'sources_sha256': tested}
    lab.write_json(BUILD / 'report.json', report)
    print(f'PalettedContainer parity: {len(scenarios)} scenarios, {edits:,} edits, all hashes identical')


if __name__ == '__main__':
    main()
