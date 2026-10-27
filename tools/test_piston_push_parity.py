#!/usr/bin/env python3
"""Compare moving-piston push geometry and predicates with pinned vanilla.

`PistonMovingBlockEntity.moveCollidedEntities` decides how far an entity is
pushed; the movement it describes is `Entity.move(MoverType.PISTON, ...)`, whose
collision and per-tick limit the game owns. This harness covers the deciding
half: the movement area, the offset block box, `getMovement` on every face, the
per-box loop, `moveStuckEntities`' box and `matchesStickyCritera`, and
`fixEntityWithinPistonBase`.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    def f32(v): return struct.pack('>f', float(v)).hex()
    def f64(v): return struct.pack('>d', float(v)).hex()
    def box(values): return [f64(v) for v in values]
    step = [1.0 / 3.0, 0.25, 0.5, 0.75, 1.0]
    unit = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
    offsets = [[0, 0, 0], [-17, -64, 30000000], [2147483647, -2147483648, -2147483648]]
    faces = [0, 1, 2, 3, 4, 5]
    # Progress pairs around the 0.25/0.5/1.0 thresholds the head and the
    # extended-progress offset change at.
    progress = [0.0, 0.25, 0.49999997, 0.5, 0.75, 0.99999994, 1.0]
    entities = [
        [0.9, 0.0, 0.2, 1.5, 1.8, 0.8],
        [1.4, 0.0, 0.2, 2.0, 1.8, 0.8],
        [0.75, 0.0, 0.2, 1.35, 1.8, 0.8],
        [3.0, 0.0, 0.2, 3.6, 1.8, 0.8],
        [0.49, 0.0, 0.49, 0.51, 1.8, 0.51],
        [0.0, 1.0, 0.0, 1.0, 2.8, 1.0],
    ]
    for index, position in enumerate(offsets):
        for direction in faces:
            for extending in (False, True):
                for source in (False, True):
                    for current in progress:
                        advanced = min(current + 0.5, 1.0)
                        for entity in entities:
                            yield {
                                'pos': position,
                                'direction': direction,
                                'extending': extending,
                                'source': source,
                                'progress': f32(current),
                                'new_progress': f32(advanced),
                                'block_box': box(unit),
                                'movement_box': box(unit),
                                'entity': box(entity),
                                'boxes': [box(unit)],
                                'sticky_top': f64(0.0),
                                'reaction': 0,
                                'on_ground': True,
                                'supported': False,
                                'entity_x': f64(entity[0]),
                                'entity_z': f64(entity[2]),
                            }
    # Split boxes, a retracting honey drag with every push reaction, and the
    # predicate's supported/on-ground corners.
    for index in range(4000):
        rng = random.Random(0x50555348 + index)
        current = rng.choice(progress)
        advanced = min(current + 0.5, 1.0)
        count = rng.randint(1, 4)
        boxes = []
        for _ in range(count):
            low = [rng.choice([0.0, 0.25, 0.5]), rng.choice([0.0, 0.5]), rng.choice([0.0, 0.5])]
            high = [min(v + rng.choice([0.25, 0.5, 1.0]), 1.0) for v in low]
            boxes.append(box(low + high))
        yield {
            'pos': rng.choice(offsets),
            'direction': rng.randrange(6),
            'extending': rng.random() < 0.5,
            'source': rng.random() < 0.5,
            'progress': f32(current),
            'new_progress': f32(advanced),
            'block_box': boxes[0],
            'movement_box': boxes[rng.randrange(len(boxes))],
            'entity': box([
                rng.choice([0.0, 0.49, 0.75, 0.9, 1.4]),
                rng.choice([0.0, 0.5]),
                rng.choice([0.0, 0.2, 0.49]),
                rng.choice([0.51, 1.35, 1.5, 2.0]),
                rng.choice([1.8, 2.8]),
                rng.choice([0.51, 0.8, 1.0]),
            ]),
            'boxes': boxes,
            'sticky_top': f64(rng.choice([0.0, 0.5, 0.9375])),
            'reaction': rng.randrange(5),
            'on_ground': rng.random() < 0.5,
            'supported': rng.random() < 0.5,
            'entity_x': f64(rng.choice([0.0, 0.5, 1.0, 2.0])),
            'entity_z': f64(rng.choice([0.0, 0.5, 1.0, 2.0])),
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-piston-push-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPistonPushOracle.java'
    helpers = [oracle.with_name(name + '.java')
               for name in ['VanillaCollisionOracle', 'VanillaBlockStateCodecOracle', 'VanillaFaceSupportOracle']]
    sources = [oracle, *helpers, Path(__file__), lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-core/Cargo.toml', lab.ROOT / 'crates/horizon-world/Cargo.toml',
               *sorted((lab.ROOT / 'crates/horizon-core/src').rglob('*.rs')),
               *sorted((lab.ROOT / 'crates/horizon-world/src').rglob('*.rs')),
               lab.ROOT / 'crates/horizon-world/examples/probe/piston_push_fixture.rs']
    tested_sources = base.source_hashes(sources)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)

    dependency_path = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
         '-sourcepath', str(lab.ROOT / 'mods/horizon/src/test/java'),
         '-cp', str(jars['server']) + os.pathsep + dependency_path,
         '-d', classes, oracle, *helpers], 'compile-java')

    def java(side, mode, path):
        run(['java', '-Xmx512M', '-cp', os.pathsep.join([str(classes), str(jars[side]), dependency_path]),
             'dev.horizonlab.parity.VanillaPistonPushOracle', mode, path], side + '-' + mode)

    fixtures = list(corpus())
    inputs = build / 'inputs.txt'
    inputs.write_text(''.join(json.dumps(value, separators=(',', ':')) + '\n' for value in fixtures), encoding='utf-8')
    fixture_paths = [inputs]
    tested_fixtures = base.source_hashes(fixture_paths)
    run(['cargo', 'build', '--release', '-p', 'horizon-world', '--example', 'world_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release/examples' / ('world_probe.exe' if os.name == 'nt' else 'world_probe')
    run([binary, 'piston_push_fixture', build], 'rust')
    references = []
    for side, jar in jars.items():
        java(side, side, build)
        reference = build / f'{side}.txt'
        matched = base.compare(reference, build / 'rust.txt')
        references.append({'side': side, 'jar_sha256': lab.digest(jar, 'sha256'),
                           'output_sha256': lab.digest(reference, 'sha256'), 'matched_output_lines': matched})
        print(f'{side}: {matched:,} piston-push output lines match Rust', flush=True)
    base.ensure_sources_unchanged(sources, tested_sources)
    base.ensure_sources_unchanged(fixture_paths, tested_fixtures)
    report = {
        'schema': 1,
        'minecraft': lock['minecraft']['version'],
        'manifest_sha1': lock['minecraft']['manifest_sha1'],
        'platform': platform.platform(),
        'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'fixtures': len(fixtures),
        'sources_sha256': tested_sources,
        'fixtures_sha256': tested_fixtures,
        'rust_output_sha256': lab.digest(build / 'rust.txt', 'sha256'),
        'references': references,
        'scope': 'Actual PistonMovingBlockEntity push geometry and predicates: getExtendedProgress, '
                 'moveByPositionAndProgress, PistonMath.getMovementArea, getMovement on every face, the '
                 'moveCollidedEntities loop over split shapes with its delta cap and PUSH_OFFSET, the '
                 'getEntities search box, and moveStuckEntities\' box with matchesStickyCritera for every push '
                 'reaction. fixEntityWithinPistonBase is covered by unit tests only: it calls '
                 'Entity.move(MoverType.PISTON, ...) and so needs a live level. Entity selection order, that '
                 'collision pass, the per-tick 0.51 limit, block behaviour of the moved state and full gameplay '
                 'remain elsewhere.',
    }
    lab.write_json(report_path, report)
    print(f'Scoped piston-push parity passed: {report_path}')


if __name__ == '__main__':
    main()