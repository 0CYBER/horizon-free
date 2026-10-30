#!/usr/bin/env python3
"""Compare registered effect particle/visibility synchronization and native data identity."""
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
    effects = catalog['effects']
    count = len(effects)
    invisibility = next(i for i, e in enumerate(effects) if e['id'] == 'minecraft:invisibility')
    glowing = next(i for i, e in enumerate(effects) if e['id'] == 'minecraft:glowing')
    constant = next(i for i, e in enumerate(effects) if e['factory'] == 'constant')
    color = next(i for i, e in enumerate(effects) if e['factory'] == 'color')

    def op(mode, **kw):
        result = dict(mode=mode, target=0, key=0, pool=0, flags=0, bit=5, flag=True,
                      ambient=False, visible=True, fail=-1, mutate=-1, changes={}, actions=[])
        result.update(kw)
        return result

    def row(pool, setup, ops, **kw):
        result = dict(pool=pool, flags=[0, 64], ambient=[False, True], controls={}, setup=setup, ops=ops)
        result.update(kw)
        return result

    def instance(effect, ambient=False, visible=True, amplifier=0):
        return dict(effect=effect, ambient=ambient, visible=visible, amplifier=amplifier)

    for key in range(count):
        for ambient in [False, True]:
            for visible in [False, True]:
                yield row([instance(key, ambient, visible)], [op('put', key=key)],
                          [op('dirty-update'), op('pack'), op('dirty-update'), op('controls', changes={'dirty': True}),
                           op('dirty-update'), op('pack'), op('particles'), op('pack'), op('all-ambient'),
                           op('details', ambient=not ambient, visible=not visible), op('invisibility'), op('pack'),
                           op('remove-particles'), op('clear'), op('invisibility'), op('defaults')])
    for flags in range(256):
        for bit in [-2**31, -33, -1, 0, 5, 6, 7, 8, 31, 32, 63, 2**31-1]:
            for flag in [False, True]:
                yield row([], [], [op('set-flag', bit=bit, flag=flag), op('get-flag', bit=bit), op('pack')], flags=[flags, 255-flags])

    pool = [instance(i, bool(i % 2), i % 3 != 0, i * 7) for i in range(count)]
    patterns = [[], [invisibility], [glowing], [constant], [color, constant, invisibility, glowing], list(range(count))]
    actions = [[], [op('clear')], [op('drop', key=invisibility), op('put', key=glowing, pool=glowing)],
               [op('details', pool=color, ambient=True, visible=True), op('put', key=color, pool=color)],
               [op('put', key=invisibility, pool=constant)], [op('pack')],
               [op('flags', flags=255), op('ambient', ambient=True)],
               [op('flags', target=1, flags=32), op('pack', target=1)]]
    for pattern_index, pattern in enumerate(patterns):
        setup = [op('put', key=key, pool=key) for key in pattern]
        for mode in ['dirty-update', 'invisibility', 'particles', 'glowing-update', 'glowing', 'glow-tag', 'remove-particles']:
            for at in range(-1, 24):
                yield row(pool, setup, [op(mode, fail=at), op('pack'), op('defaults'), op('dirty-update')], controls={'client': bool(pattern_index % 2)})
                for action in actions:
                    yield row(pool, setup, [op(mode, mutate=at, changes={'data': 1, 'client': not bool(pattern_index % 2), 'tag': True, 'dirty': True}, actions=action),
                                           op('pack'), op('pack', target=1), op('dirty-update')], controls={'client': bool(pattern_index % 2)})
    rng = random.Random(0x4546464543545359)
    for _ in range(250):
        state = [instance(i, bool(rng.randrange(2)), bool(rng.randrange(2)), rng.choice([-1, 0, 1, 255, 256])) for i in range(count)]
        setup = [op('put', key=key, pool=key) for key in rng.sample(range(count), rng.randrange(count+1))]
        ops = []
        for _ in range(30):
            ops.append(op(rng.choice(['dirty-update', 'particles', 'invisibility', 'glowing-update', 'glow-tag', 'invisible',
                                     'put', 'drop', 'clear', 'details', 'pack', 'defaults', 'all-ambient', 'flags', 'ambient', 'controls']),
                          key=rng.randrange(count), pool=rng.randrange(count), target=rng.randrange(2), flags=rng.randrange(256),
                          ambient=bool(rng.randrange(2)), visible=bool(rng.randrange(2)), flag=bool(rng.randrange(2)),
                          fail=rng.choice([-1, -1, -1, 0, 1, 2]), mutate=rng.choice([-1, -1, 0, 1, 2, 3]),
                          changes={'data': rng.randrange(2), 'client': bool(rng.randrange(2)), 'tag': bool(rng.randrange(2)), 'dirty': bool(rng.randrange(2))}, actions=rng.choice(actions)))
        yield row(state, setup, ops, flags=[rng.randrange(256), rng.randrange(256)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-effect-sync-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEffectSyncOracle.java'
    helper = oracle.with_name('VanillaEffectParticlesOracle.java')
    sources = [oracle, helper, Path(__file__),
               lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-core/Cargo.toml', lab.ROOT / 'crates/horizon-world/Cargo.toml',
               *sorted((lab.ROOT / 'crates/horizon-core/src').rglob('*.rs')),
               *sorted((lab.ROOT / 'crates/horizon-world/src').rglob('*.rs')),
               lab.ROOT / 'crates/horizon-world/examples/probe/effect_sync_fixture.rs']
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
             'dev.horizonlab.parity.VanillaEffectSyncOracle', mode, build], mode)

    for side in jars:
        java(side, 'export-' + side)
    catalog = json.loads((build / 'catalog-server.json').read_text())
    if catalog != json.loads((build / 'catalog-client.json').read_text()):
        raise AssertionError('Client/server effect particle catalogs differ')
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
        run([binary, 'effect_sync_fixture', build, side], 'rust-' + side)
        reference = build / f'{side}.txt'
        candidate = build / f'rust-{side}.txt'
        matched = base.compare(reference, candidate)
        references.append(dict(side=side, jar_sha256=lab.digest(jar, 'sha256'),
                               output_sha256=lab.digest(reference, 'sha256'), matched_output_lines=matched,
                               rust_output_sha256=lab.digest(candidate, 'sha256')))
        print(f'{side}: {matched:,} effect synchronization output lines match Rust', flush=True)
    base.ensure_sources_unchanged(sources, tested_sources)
    base.ensure_sources_unchanged(fixture_paths, tested_fixtures)
    report = dict(schema=1, minecraft=lock['minecraft']['version'], manifest_sha1=lock['minecraft']['manifest_sha1'],
                  platform=platform.platform(), java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(), fixtures=len(fixtures),
                  registered_effects=len(catalog['effects']), constant_factories=sum(e['factory'] == 'constant' for e in catalog['effects']),
                  fastutil_sha256={str(p.relative_to(lab.ROOT)): lab.digest(p, 'sha256') for p in libraries if 'fastutil' in str(p)},
                  sources_sha256=tested_sources, fixtures_sha256=tested_fixtures, references=references,
                  scope='Inherited native LivingEntity effect particle/invisibility/glowing synchronization, dirty clearing, Entity shared byte flags and actual SynchedEntityData mixed byte/boolean/particle-list storage. All 40 registered effects use exporter-checked inherited pure factories: fresh identity-bearing colored options or registry-constant simple options. Tests active-map order, particle option identities/list equality, ambient/visible flags, server/client glowing branches, repeated virtual lookups, captured data objects, reentrant data packing/writes, callback map mutations and failure prefixes. Replays each JVM holder hashes/identities. Custom impure particle factories, particle rendering/transport, full effect tick integration and complete gameplay/platform builds remain outside this gate.')
    lab.write_json(report_path, report)
    print(f'Scoped effect synchronization parity passed: {report_path}')


if __name__ == '__main__':
    main()
