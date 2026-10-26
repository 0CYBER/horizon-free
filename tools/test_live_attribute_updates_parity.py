#!/usr/bin/env python3
"""Compare live registered attributes with native entity notifications and update consequences."""
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
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(catalog):
    def bits(value):
        return struct.pack('>d', float(value)).hex()

    def fl(value):
        return int.from_bytes(struct.pack('>f', float(value)), 'big')

    def op(mode, attribute=0, **kw):
        result = dict(mode=mode, target=0, attribute=attribute, id='horizon:a',
                      amount=bits(.25), base=bits(7), operation=0, value=fl(100),
                      amplifier=0, fail=-1, mutate=-1, changes={}, actions=[],
                      reapply=True, count=1, flag=False)
        result.update(kw)
        return result

    def row(setup, ops, entity='minecraft:player', **kw):
        result = dict(entity=entity, health=[fl(100), fl(200)], absorption=fl(100), setup=setup, ops=ops)
        result.update(kw)
        return result

    attrs = catalog['attributes']
    count = len(attrs)
    ids = [next(i for i, a in enumerate(attrs) if a['id'] == 'minecraft:' + name)
           for name in ['max_health', 'max_absorption', 'scale', 'waypoint_transmit_range', 'attack_damage']]
    values = [-math.inf, -1e30, -1, -0., 0., 1e-40, .1, 1, 2, 20, 1e30, math.inf, math.nan]
    rng = random.Random(0x4c495645555044)
    for entity in catalog['entities']:
        for reverse in [False, True]:
            order = list(range(count))
            if reverse:
                order.reverse()
            ops = [op(mode, i) for i in [-1, *order] for mode in ['value', 'base']]
            for target in range(2):
                ops.extend(op('ensure', i, target=target) for i in order)
            ops += [op('refresh'), op('pack'), op('pack', target=1), op('controls', changes={'selection': 1}), op('refresh')]
            for i in order:
                ops.append(op('define', i, operation=i % 3, amount=bits(-2)))
            ops += [op('dirty'), op('notify-add', amplifier=1), op('refresh'), op('notify-update', amplifier=2),
                    op('notify-remove'), op('notify-remove', count=0), op('controls', changes={'client': True}),
                    op('dirty'), op('notify-add'), op('notify-update'), op('notify-remove'),
                    op('controls', changes={'client': False, 'server': False}), op('attribute-update', ids[3])]
            yield row([], ops, entity)

    setup = [op('set', key, target=target, base=bits(10 if target == 0 else 30))
             for target in range(2) for key in ids]
    setup += [op('define', key, amount=bits(-3)) for key in ids]
    for key in ids[:4]:
        for value in values:
            for operation in range(3):
                ops = [op('set', key, base=bits(value)), op('attribute-update', key), op('pack'),
                       op('define', key, amount=bits(value), operation=operation), op('notify-add'),
                       op('refresh'), op('set-health', value=fl(value)), op('set-absorption', value=fl(value)),
                       op('notify-update', amplifier=255), op('notify-remove'), op('pack'), op('refresh')]
                yield row(setup, ops, health=[fl(value), fl(-value)], absorption=fl(value))

    changes = {'selection': 1, 'direct': 1, 'data': 1, 'manager': 1, 'absorption': fl(50)}
    actions = [[], [op('set', ids[0], base=bits(2)), op('set', ids[0], target=1, base=bits(3))],
               [op('set', ids[1], base=bits(2)), op('set', ids[1], target=1, base=bits(3))],
               [op('clear-updates')], [op('null-update')], [op('clear-updates', target=1)],
               [op('set', ids[3], target=1, base=bits(0))], [op('clear-sync')]]
    # Inject failures and routing/limit changes at successive callback positions
    # so captures and writes preceding an exception remain visible in state.
    for key in ids[:4]:
        for mode in ['attribute-update', 'refresh', 'set-health', 'set-absorption', 'notify-add', 'notify-update', 'notify-remove']:
            for at in range(-1, 32):
                yield row(setup, [op(mode, key, fail=at), op('pack'), op('refresh')])
                for action in actions:
                    yield row(setup, [op(mode, key, mutate=at, changes=changes, actions=action), op('pack'), op('refresh')])
    large_setup = [op('ensure', key, target=target) for target in range(2) for key in range(count)]
    large_setup += [op('define', key, amount=bits(-3)) for key in range(count)]
    for mode in ['refresh', 'notify-update', 'notify-remove']:
        for at in range(64):
            yield row(large_setup, [op(mode, fail=at), op('pack'), op('refresh')])
            yield row(large_setup, [op(mode, mutate=at, changes=changes, actions=actions[at % len(actions)]), op('pack'), op('refresh')])
    for amp in [-2**31, -1, 0, 1, 255, 256, 2**31-1]:
        for reapply in [False, True]:
            for n in [0, 1, 3]:
                yield row(setup, [op('notify-add', amplifier=amp), op('notify-update', amplifier=amp, reapply=reapply),
                                  op('notify-remove', count=n), op('refresh'), op('pack')])
    for _ in range(200):
        ops = []
        for _ in range(40):
            ops.append(op(rng.choice(['set', 'update', 'permanent', 'remove', 'reset', 'value', 'base',
                                     'refresh', 'attribute-update', 'define', 'notify-add', 'notify-update',
                                     'notify-remove', 'effect-add', 'effect-remove', 'pack', 'set-health',
                                     'set-absorption', 'heal', 'clear-updates', 'clear-sync']),
                          rng.choice([-1, *ids]), target=rng.randrange(2), operation=rng.randrange(3),
                          amount=bits(rng.choice(values)), base=bits(rng.choice(values)), value=fl(rng.choice(values)),
                          amplifier=rng.choice([-2**31, -1, 0, 1, 255, 256, 2**31-1]), count=rng.randrange(3),
                          reapply=bool(rng.randrange(2)), fail=rng.choice([-1, -1, -1, 0, 1, 2]),
                          mutate=rng.choice([-1, -1, 0, 1, 2]), changes=changes, actions=rng.choice(actions)))
        yield row(setup, ops)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-live-attribute-updates-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLiveAttributeUpdatesOracle.java'
    helper = oracle.with_name('VanillaEntityAttributeOracle.java')
    live_helper = oracle.with_name('VanillaLiveAttributesOracle.java')
    sources = [oracle, helper, live_helper, Path(__file__),
               lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-core/Cargo.toml', lab.ROOT / 'crates/horizon-world/Cargo.toml',
               *sorted((lab.ROOT / 'crates/horizon-core/src').rglob('*.rs')),
               *sorted((lab.ROOT / 'crates/horizon-world/src').rglob('*.rs')),
               lab.ROOT / 'crates/horizon-world/examples/probe/live_attribute_updates_fixture.rs',
               lab.ROOT / 'crates/horizon-world/examples/probe/live_attributes_fixture.rs']
    tested_sources = base.source_hashes(sources)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)

    dependency_path = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp',
         str(jars['server']) + os.pathsep + dependency_path, '-d', classes, oracle, helper, live_helper], 'compile-java')

    def java(side, mode):
        run(['java', '-Xmx512M', '-cp', os.pathsep.join([str(classes), str(jars[side]), dependency_path]),
             'dev.horizonlab.parity.VanillaLiveAttributeUpdatesOracle', mode, build], mode)

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
        run([binary, 'live_attribute_updates_fixture', build, side], 'rust-' + side)
        reference = build / f'{side}.txt'
        candidate = build / f'rust-{side}.txt'
        matched = base.compare(reference, candidate)
        references.append(dict(side=side, jar_sha256=lab.digest(jar, 'sha256'),
                               output_sha256=lab.digest(reference, 'sha256'), matched_output_lines=matched,
                               rust_output_sha256=lab.digest(candidate, 'sha256')))
        print(f'{side}: {matched:,} live attribute update output lines match Rust', flush=True)
    base.ensure_sources_unchanged(sources, tested_sources)
    base.ensure_sources_unchanged(fixture_paths, tested_fixtures)
    report = dict(schema=1, minecraft=lock['minecraft']['version'], manifest_sha1=lock['minecraft']['manifest_sha1'],
                  platform=platform.platform(), java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(), fixtures=len(fixtures),
                  attributes=len(catalog['attributes']), entity_suppliers=len(catalog['entities']),
                  fastutil_sha256={str(p.relative_to(lab.ROOT)): lab.digest(p, 'sha256') for p in libraries if 'fastutil' in str(p)},
                  sources_sha256=tested_sources, fixtures_sha256=tested_fixtures, references=references,
                  scope='Native LivingEntity attribute queries, notification and private refresh methods with registered AttributeMaps, real effect modifiers and pending queues, onAttributeUpdated health/absorption consequences and SynchedEntityData Float storage. Exercises all registered attributes/default suppliers, separate captured/selected maps and health stores, callbacks changing limits and routes, null pending members, client guards, failure prefixes and modifier/cache/queue identity. Replays JVM holder/instance hashes per reference. Scale dimension rebuild and waypoint-manager internals remain controlled; passenger lists are empty. Full effect lifecycle/dirty-particle synchronization, player absorption, complete gameplay and playable platform builds remain pending.')
    lab.write_json(report_path, report)
    print(f'Scoped live attribute update parity passed: {report_path}')


if __name__ == '__main__':
    main()
