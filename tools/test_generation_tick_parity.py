#!/usr/bin/env python3
"""Compare proto-chunk ticks and world-generation tick routing with vanilla."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab, test_rust_parity as base


def corpus():
    extremes = [-9223372036854775808, -2147483648, -1, 0, 1, 2147483647, 9223372036854775807]
    for time in extremes:
        yield 'reset'
        yield 'load -1 -1 4 0 -1 64 -1 -2147483648 -100 0 -1 64 -1 2147483647 100 1 -1 64 -1 -1 0 2 -2 65 -2 12 3'
        yield 'query 0 -1 64 -1'
        yield 'snapshot -1 -1'
        yield f'schedule 0 -1 64 -1 {time} -3 {time}'
        yield f'schedule 3 -3 66 -3 {time} 2 {time}'
        yield f'dump -1 -1 {time}'
        yield 'snapshot_dump'
        yield f'promote -1 -1 {time}'
        yield f'dump -1 -1 {time}'
        yield f'schedule 4 -4 67 -4 {time} -2 {time}'
        yield f'dump -1 -1 {time}'
        yield 'snapshot_dump'
    for kind in ['load', 'level']:
        yield 'reset'
        # Deliberately load a position outside the owning container; routing is
        # determined by each request's position, not by entries in saved lists.
        yield f'{kind} 0 0 2 0 16 64 0 7 0 0 16 64 0 8 -3'
        yield 'query 0 16 64 0'
        yield 'dump 0 0 0'
        yield 'dump 1 0 0'
        yield 'fail 1'
        yield 'query 0 0 64 0'
        yield 'schedule 1 0 64 0 0 0 0'
        yield 'will 1 0 64 0'
        yield 'count'
        yield 'fail 0'
        yield 'query 1 0 64 0'
        yield 'unpack 0 0 9223372036854775807'
        yield 'dump 0 0 -9223372036854775808'
    positions = [[-2147483648, -2147483648, 2147483647], [2147483647, 2147483647, -2147483648],
                 [0, 0, 0], [67108864, 4096, -67108864], [-1, -1, -1], [15, 319, 15], [16, -64, 16]]
    for priority in [-2147483648, -4, -3, -2, -1, 0, 1, 2, 3, 4, 2147483647]:
        yield 'reset'
        for i, p in enumerate(positions):
            xyz = ' '.join(map(str, p))
            yield f'schedule {i} {xyz} -9223372036854775808 {priority} 9223372036854775807'
            yield f'schedule {i} {xyz} 9223372036854775807 {-priority if priority != -2147483648 else 0} -9223372036854775808'
            yield f'query {i} {xyz}'
            yield f'will {i} {xyz}'
            yield f'dump {p[0] >> 4} {p[2] >> 4} 12345'
    rng = random.Random(0x484f52495a4f4e)
    for trial in range(12):
        yield 'reset'
        for cx in range(-2, 3):
            command = 'load' if (cx + trial) % 2 == 0 else 'level'
            rows = []
            for i in range(24):
                row = [rng.randrange(8), cx*16+rng.randrange(4), rng.randrange(-3, 4), rng.randrange(4), rng.choice([-2147483648, -1, 0, 1, 2147483647]), rng.randrange(-8, 9)]
                rows.append(' '.join(map(str, row)))
                if i % 3 == 0: rows.append(rows[-1])
            yield f'{command} {cx} 0 {len(rows)} ' + ' '.join(rows)
        for step in range(700):
            cx = rng.randrange(-2, 3); p = [cx*16+rng.randrange(4), rng.randrange(-3, 4), rng.randrange(4)]
            xyz = ' '.join(map(str, p)); typ = rng.randrange(8)
            if step % 71 == 0:
                yield 'fail 1'; yield f'query {typ} {xyz}'; yield f'will {typ} {xyz}'; yield 'count'; yield 'fail 0'
            if step % 79 == 0: yield f'promote {cx} 0 {rng.choice(extremes)}'
            if step % 83 == 0: yield f'snapshot {cx} 0'
            if step % 89 == 0: yield 'snapshot_dump'
            if step % 101 == 0: yield f'remove {cx} 0'
            yield f'schedule {typ} {xyz} {rng.choice(extremes)} {rng.randrange(-8, 9)} {rng.choice(extremes)}'
            yield f'query {typ} {xyz}'
            if step % 23 == 0: yield f'dump {cx} 0 {rng.choice(extremes)}'
        for cx in range(-2, 3): yield f'dump {cx} 0 {rng.choice(extremes)}'
        yield 'snapshot_dump'


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--prepare', action='store_true'); args = parser.parse_args()
    build = lab.LOCAL / 'rust-generation-tick-parity'; build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'; report_path.unlink(missing_ok=True)
    if args.prepare: lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaGenerationTickOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py', lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock', lab.ROOT / 'crates/horizon-core/Cargo.toml',
               *sorted((lab.ROOT / 'crates/horizon-core/src').rglob('*.rs')), lab.ROOT / 'crates/horizon-core/examples/generation_tick_fixture.rs']
    tested_sources = base.source_hashes(sources)
    commands = list(corpus()); inputs = build / 'inputs.txt'; inputs.write_text('\n'.join(commands) + '\n')
    tested_fixtures = base.source_hashes([inputs]); classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, libraries))
    def run(command, name, **kwargs):
        with (build / (name + '.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600, stderr=log, **({'stdout': log} | kwargs))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', str(jars['server']) + os.pathsep + dependencies, '-d', classes, oracle], 'compile-java')
    run(['cargo', 'build', '--release', '-p', 'horizon-core', '--example', 'generation_tick_fixture'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release/examples' / ('generation_tick_fixture.exe' if os.name == 'nt' else 'generation_tick_fixture')
    with inputs.open('rb') as source, (build / 'rust.txt').open('wb') as output: run([binary], 'rust', stdin=source, stdout=output)
    references = []
    for side, jar in jars.items():
        run(['java', '-Xmx512M', '-cp', os.pathsep.join([str(classes), str(jar), dependencies]), 'dev.horizonlab.parity.VanillaGenerationTickOracle', inputs, build / (side + '.txt')], side)
        matched = base.compare(build / (side + '.txt'), build / 'rust.txt')
        references.append({'side': side, 'jar_sha256': lab.digest(jar, 'sha256'), 'output_sha256': lab.digest(build / (side + '.txt'), 'sha256'), 'matched_output_lines': matched})
        print(f'{side}: {matched:,} generation-tick lines match Rust', flush=True)
    base.ensure_sources_unchanged(sources, tested_sources); base.ensure_sources_unchanged([inputs], tested_fixtures)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'], 'platform': platform.platform(),
              'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0], 'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'commands': dict(Counter(command.split()[0] for command in commands)), 'sources_sha256': tested_sources, 'fixtures_sha256': tested_fixtures,
              'rust_output_sha256': lab.digest(build / 'rust.txt', 'sha256'), 'references': references,
              'scope': 'Native ProtoChunkTicks load/schedule deduplication, insertion order, zero delay for scheduled requests, preserved saved delays/priorities, time-independent packing and independent scheduledTicks snapshots. Native WorldGenTickAccess lookup ordering, routing to mixed proto/level containers, lookup failures, false willTickThisTick and zero count without lookups. Proto-to-level promotion and unpacked save behavior, position/type identities, priorities, duplicates, unloaded containers and integer/time extremes are compared. Rust exposes pack as a borrowed immutable slice; mutation of the native returned list is outside the API. Actual block/fluid execution and feature-world scheduling integration remain pending.'}
    lab.write_json(report_path, report); print(f'Scoped generation-tick parity passed: {report_path}')


if __name__ == '__main__': main()
