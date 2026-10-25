#!/usr/bin/env python3
"""Compare the structure starts Horizon saves with the tags vanilla's StructureStart.createTag writes.

The oracle (VanillaStructureStartOracle with HORIZON_START_NBT) generates the start of each case's chunk with the
pinned server's own classes and prints its tag; the probe (world_probe structure_start_nbt) prints the tag
Horizon saves for the same chunk. Both print sorted keys and typed numbers, and the lines must be identical.
The cases are the start chunks of one site of every structure kind (seed 12345, and a badlands mineshaft of seed
-7777), which between them use every StructurePieceType.
"""
import argparse
import collections
import json
import os
from pathlib import Path
import platform
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

# (seed, dimension, chunk x, chunk z): the start chunk of one site of each structure kind.
CASES = [
    (12345, 'overworld', -9, -52), (12345, 'overworld', -146, 37), (12345, 'overworld', -66, -98),
    (12345, 'overworld', 91, 75), (12345, 'overworld', 75, -23), (12345, 'overworld', 17, -60),
    (12345, 'overworld', 11, -11), (12345, 'overworld', 91, -90), (12345, 'overworld', 8, -33),
    (12345, 'overworld', 2, -71), (12345, 'overworld', 39, -28), (12345, 'overworld', -90, -60),
    (12345, 'the_nether', 11, -16),
    (12345, 'overworld', 291, 441), (12345, 'overworld', 36, 41), (12345, 'overworld', -8, 2),
    (12345, 'overworld', 114, -133), (12345, 'overworld', -16, 5), (12345, 'overworld', -109, 91),
    (12345, 'overworld', 0, -18), (12345, 'overworld', -150, 51), (12345, 'overworld', 61, 49),
    (12345, 'the_nether', 0, -18), (12345, 'the_nether', -22, 6), (12345, 'the_end', -15, -76),
    (12345, 'overworld', 38, -115), (12345, 'overworld', -238, -313), (12345, 'overworld', -95, 2),
    (12345, 'overworld', -126, -106), (12345, 'overworld', 47, 25), (12345, 'overworld', 110, 23),
    (12345, 'overworld', 4, -1), (12345, 'the_nether', -13, -37), (-7777, 'overworld', 75, 84),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    build = lab.LOCAL / 'rust-structure-start-nbt-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    report_path.unlink(missing_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStructureStartOracle.java'
    probe = lab.ROOT / 'crates/horizon-world/examples/probe/structure_start_nbt.rs'
    sources = [oracle, probe, Path(__file__), lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock', lab.ROOT / 'crates/horizon-world/Cargo.toml',
               *sorted((lab.ROOT / 'crates/horizon-core/src').rglob('*.rs')),
               *sorted((lab.ROOT / 'crates/horizon-world/src').rglob('*.rs'))]
    tested_sources = base.source_hashes(sources)

    def run(command, name, env=None):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT, env=env)

    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath',
         lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle], 'compile-java')
    run(['cargo', 'build', '--release', '-p', 'horizon-world', '--example', 'world_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release/examples' / (
        'world_probe.exe' if os.name == 'nt' else 'world_probe')
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    groups = collections.defaultdict(list)
    for seed, dimension, cx, cz in CASES:
        groups[(seed, dimension)].append((cx, cz))
    vanilla, horizon = [], []
    for (seed, dimension), chunks in sorted(groups.items()):
        name = f'{seed}-{dimension}'
        out = build / f'client-{name}.txt'
        env = dict(os.environ, HORIZON_START_NBT='1', HORIZON_DIMENSION=dimension,
                   HORIZON_CHUNKS=';'.join(f'{x},{z}' for x, z in chunks))
        run(['java', '-Xmx2g', '-cp', str(classes) + os.pathsep + dependencies,
             'dev.horizonlab.parity.VanillaStructureStartOracle', out, seed, 0, 0, 0, 0], 'client-' + name, env)
        vanilla += sorted(l for l in out.read_text(encoding='utf-8').splitlines() if l.startswith('nbt '))
        probe_out = subprocess.check_output([str(binary), 'structure_start_nbt', str(resources), str(seed), dimension,
                                             *(f'{x},{z}' for x, z in chunks)], text=True)
        horizon += sorted(probe_out.splitlines())
    (build / 'client.txt').write_text(''.join(l + '\n' for l in vanilla), encoding='utf-8')
    (build / 'rust.txt').write_text(''.join(l + '\n' for l in horizon), encoding='utf-8')
    if len(vanilla) != len(CASES):
        raise AssertionError(f'the oracle found {len(vanilla)} starts for {len(CASES)} cases')
    matched = base.compare(build / 'client.txt', build / 'rust.txt')
    base.ensure_sources_unchanged(sources, tested_sources)
    lab.write_json(report_path, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(), units=len(CASES),
        sources_sha256=tested_sources, matched_output_lines=matched,
        output_sha256=lab.digest(build / 'rust.txt', 'sha256'), client_jar_sha256=lab.digest(jars['client'], 'sha256')))
    print(f'{len(CASES)} structure starts save as vanilla ({matched} lines): {report_path}', flush=True)


if __name__ == '__main__':
    main()
