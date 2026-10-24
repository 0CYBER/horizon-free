#!/usr/bin/env python3
"""Compare registered live attribute maps, queues and effect modifiers with vanilla."""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import random
import struct
import subprocess

try:
    from . import lab, test_rust_parity as base, test_entity_attribute_parity as entity_base
except ImportError:
    import lab
    import test_rust_parity as base
    import test_entity_attribute_parity as entity_base


def corpus(catalog):
    yield from entity_base.corpus(catalog)

    def bits(value):
        return struct.pack('>d', float(value)).hex()

    def op(mode, attribute=0, **kwargs):
        result = dict(mode=mode, attribute=attribute, id='horizon:a', amount=bits(.125),
                      operation=0, base=bits(7), amplifier=0)
        result.update(kwargs)
        return result

    count = len(catalog['attributes'])
    rng = random.Random(0x4c49564541545452)
    values = [-math.inf, -1e30, -1, -0., 0., 1e-40, .1, 1, 2, 20, 1e16, math.inf, math.nan]
    amplifiers = [-2**31, -1, 0, 1, 255, 2**31-1]
    for entity in catalog['entities']:
        for reverse in [False, True]:
            order = list(range(count))
            if reverse:
                order.reverse()
            ops = [op(mode, -1) for mode in ['has', 'has_modifier', 'value', 'base', 'modifier', 'reset', 'ensure', 'set']]
            for i in order:
                ops.extend(op(mode, i) for mode in ['has', 'value', 'base', 'reset'])
            # The effect loop creates instances in its own native map order.
            for i in [*order, -1]:
                ops.append(op('define', i, operation=i % 3, amount=bits(values[i % len(values)])))
            for amplifier in amplifiers:
                ops.extend([op('clear_updates'), op('clear_sync'), op('effect-add', amplifier=amplifier)])
                for i in order:
                    ops.extend(op(mode, i) for mode in ['value', 'modifier', 'has_modifier'])
                ops.extend([op('effect-remove'), op('effect-remove')])
            for i in order:
                ops.extend(op(mode, i) for mode in ['ack_update', 'set', 'ack_sync', 'reset', 'value'])
            ops.extend([op('clear_updates'), op('clear_sync')])
            yield dict(entity=entity, ops=ops)

    for index, attribute in enumerate(catalog['attributes']):
        entity = next(name for name, defaults in catalog['entities'].items() if attribute['id'] in defaults)
        edges = values.copy()
        for field in ['min', 'max', 'default']:
            if field in attribute:
                value = struct.unpack('>d', bytes.fromhex(attribute[field]))[0]
                edges.extend([math.nextafter(value, -math.inf), value, math.nextafter(value, math.inf)])
        for value in edges:
            ops = [op('set', index, base=bits(value)), op('base', index), op('value', index)]
            for operation in range(3):
                ops.extend([op('define', index, operation=operation, amount=bits(value)),
                            op('effect-add', amplifier=255), op('value', index),
                            op('effect-remove'), op('value', index)])
            ops.extend([op('reset', index), op('value', index)])
            yield dict(entity=entity, ops=ops)

    for _ in range(200):
        entity = rng.choice(list(catalog['entities']))
        ops = []
        for _ in range(100):
            ops.append(op(rng.choice(['define', 'effect-add', 'effect-remove', 'ensure', 'set', 'add',
                                     'permanent', 'update', 'remove', 'clear', 'value', 'base', 'modifier',
                                     'has_modifier', 'reset', 'ack_update', 'ack_sync', 'clear_updates', 'clear_sync']),
                          rng.randrange(-1, count), operation=rng.randrange(3),
                          id=rng.choice(['horizon:a', 'horizon:b', 'horizon:c']),
                          amount=bits(rng.choice(values)), base=bits(rng.choice(values)),
                          amplifier=rng.choice(amplifiers)))
        yield dict(entity=entity, ops=ops)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-live-attributes-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLiveAttributesOracle.java'
    helper = oracle.with_name('VanillaEntityAttributeOracle.java')
    sources = [oracle, helper, Path(__file__), lab.ROOT / 'tools/test_entity_attribute_parity.py',
               lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-core/Cargo.toml', lab.ROOT / 'crates/horizon-world/Cargo.toml',
               *sorted((lab.ROOT / 'crates/horizon-core/src').rglob('*.rs')),
               *sorted((lab.ROOT / 'crates/horizon-world/src').rglob('*.rs')),
               lab.ROOT / 'crates/horizon-world/examples/probe/live_attributes_fixture.rs']
    tested_sources = base.source_hashes(sources)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)

    dependency_path = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp',
         str(jars['server']) + os.pathsep + dependency_path, '-d', classes, oracle, helper], 'compile-java')

    def java(side, mode):
        run(['java', '-Xmx512M', '-cp', os.pathsep.join([str(classes), str(jars[side]), dependency_path]),
             'dev.horizonlab.parity.VanillaLiveAttributesOracle', mode, build], mode)

    for side in jars:
        java(side, 'export-' + side)
    catalog = json.loads((build / 'catalog-server.json').read_text())
    if catalog != json.loads((build / 'catalog-client.json').read_text()):
        raise AssertionError('Client/server attribute catalogs differ')
    lab.write_json(build / 'catalog.json', catalog)
    fixtures = list(corpus(catalog))
    inputs = build / 'inputs.txt'
    inputs.write_text(''.join(json.dumps(row, separators=(',', ':')) + '\n' for row in fixtures), encoding='utf-8')
    fixture_paths = [inputs, build / 'catalog.json', build / 'catalog-client.json', build / 'catalog-server.json']
    tested_fixtures = base.source_hashes(fixture_paths)
    run(['cargo', 'build', '--release', '-p', 'horizon-world', '--example', 'world_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release/examples' / ('world_probe.exe' if os.name=='nt' else 'world_probe')
    references = []
    for side, jar in jars.items():
        java(side, side)
        enriched = build / f'inputs-{side}.txt'
        fixture_paths.append(enriched)
        tested_fixtures.update(base.source_hashes([enriched]))
        run([binary, 'live_attributes_fixture', build, side], 'rust-' + side)
        reference = build / f'{side}.txt'
        candidate = build / f'rust-{side}.txt'
        matched = base.compare(reference, candidate)
        references.append(dict(side=side, jar_sha256=lab.digest(jar, 'sha256'),
                               output_sha256=lab.digest(reference, 'sha256'), matched_output_lines=matched,
                               rust_output_sha256=lab.digest(candidate, 'sha256')))
        print(f'{side}: {matched:,} live attribute output lines match Rust', flush=True)
    base.ensure_sources_unchanged(sources, tested_sources)
    base.ensure_sources_unchanged(fixture_paths, tested_fixtures)
    report = dict(schema=1, minecraft=lock['minecraft']['version'], manifest_sha1=lock['minecraft']['manifest_sha1'],
                  platform=platform.platform(), java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(), fixtures=len(fixtures),
                  attributes=len(catalog['attributes']), entity_suppliers=len(catalog['entities']),
                  fastutil_sha256={str(p.relative_to(lab.ROOT)): lab.digest(p, 'sha256') for p in libraries if 'fastutil' in str(p)},
                  sources_sha256=tested_sources, fixtures_sha256=tested_fixtures, references=references,
                  scope='Actual registered AttributeMap, immutable DefaultAttributes suppliers, ranged definitions, lazy instances, query/reset methods, modifier reference identities/caches/permanent state, MobEffect modifier loops and native-order update/sync queues. Includes the complete previous entity-attribute corpus, all registered attributes/entity suppliers, null lookups, range boundaries, nonfinite arithmetic, wrapping amplifiers and independent/bulk queue acknowledgements. Replays each reference JVM holder/instance hash sequence. Mutable custom supplier prototypes, internal supplier caches, full entity health/dimensions/gameplay, wire/save codecs and playable platform builds remain outside this gate.')
    lab.write_json(report_path, report)
    print(f'Scoped live attribute parity passed: {report_path}')


if __name__ == '__main__':
    main()
