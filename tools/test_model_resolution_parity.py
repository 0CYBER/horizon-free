#!/usr/bin/env python3
"""Compare model inheritance, missing fallback and texture aliases with the client."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_cuboid_model_parity as decoding
except ImportError:
    import lab, test_rust_parity as base, test_cuboid_model_parity as decoding


def bundle(models, roots=None):
    return {'models': {key: json.dumps(value) for key, value in models.items()},
            'roots': list(models) if roots is None else roots,
            'queries': ['', '#', 'all', '#all', 'side', '#side', 'particle', '#particle',
                        'missingno', 'unknown', 'minecraft:block/stone', '##all', '']}


def corpus():
    parent = {'ambientocclusion': False, 'gui_light': 'front', 'textures': {'side': '#all', 'all': 'block/stone'},
              'elements': [{'from': [0, 0, 0], 'to': [16, 8, 16], 'faces': {'up': {'texture': '#side'}}}],
              'display': {'thirdperson_righthand': {'rotation': [1, 2, 3]}, 'gui': {'scale': [2, 2, 2]}}}
    for child in [{}, {'elements': []}, {'ambientocclusion': True}, {'gui_light': 'side'},
                  {'display': {}}, {'display': {'gui': {}}}, {'display': {'gui': None}},
                  {'display': {'thirdperson_righthand': {'scale': [3, 2, 1]}}},
                  {'display': {'thirdperson_lefthand': {}, 'thirdperson_righthand': {'scale': [3, 2, 1]}}},
                  {'textures': {'all': 'block/dirt'}}, {'textures': {'all': '#side'}},
                  {'textures': {'side': '#absent'}}, {'textures': {'all': {'sprite': 'block/glass', 'force_translucent': True}}}]:
        yield bundle({'horizon:parent': parent, 'horizon:child': {'parent': 'horizon:parent', **child}})
    yield bundle({}, ['horizon:missing', 'minecraft:builtin/missing'])
    yield bundle({'minecraft:builtin/missing': {'elements': []}, 'horizon:child': {'parent': 'builtin/missing'}})
    for chain in [
        {'horizon:a': {'parent': 'horizon:absent'}},
        {'horizon:a': {'parent': 'horizon:a'}},
        {'horizon:a': {'parent': 'horizon:b'}, 'horizon:b': {'parent': 'horizon:a'}},
        {'horizon:a': {'parent': 'horizon:b'}, 'horizon:b': {'parent': 'horizon:b'}, 'horizon:c': {'parent': 'horizon:a', 'elements': []}},
    ]:
        yield bundle(chain)
    for tail in ['block/stone', '#absent', '#s0000', '#s0040']:
        for length in [1, 8, 64, 512]:
            textures = {f's{i:04}': f'#s{i+1:04}' for i in range(length - 1)}
            textures[f's{length-1:04}'] = tail
            textures.update({'all': '#s0000', 'side': '#all', 'particle': '#side', '': 'block/dirt', '#all': '#'})
            yield bundle({'horizon:chain': {'textures': textures}})
    # Deep parent chains use iterative traversal, including descendant cycles.
    for final in [{}, {'parent': 'horizon:n0'}, {'parent': 'horizon:missing'}]:
        models = {f'horizon:n{i}': {'parent': f'horizon:n{i+1}'} for i in range(1023)}
        models['horizon:n1023'] = final
        yield bundle(models, ['horizon:n0', 'horizon:n512', 'horizon:n1023'])
    rng = random.Random(0x5245534f4c5645)
    for _ in range(400):
        count = rng.randrange(1, 13)
        models = {}
        for i in range(count):
            model = {}
            if rng.randrange(4): model['parent'] = f'horizon:n{rng.randrange(count+2)}'
            if rng.randrange(3) == 0: model['ambientocclusion'] = rng.choice([True, False])
            if rng.randrange(3) == 0: model['gui_light'] = rng.choice(['front', 'side'])
            if rng.randrange(3) == 0: model['elements'] = rng.choice([[], [None], parent['elements']])
            if rng.randrange(3) == 0:
                model['display'] = {rng.choice(['gui', 'firstperson_righthand', 'thirdperson_lefthand', 'on_shelf']):
                                    rng.choice([None, {}, {'translation': [i, -i, 0]}])}
            model['textures'] = {key: rng.choice(['block/stone', '#all', '#side', '#missingno', '#', '#absent',
                                                {'sprite': 'block/glass', 'force_translucent': True}])
                                 for key in rng.sample(['all', 'side', 'particle', '', 'missingno'], rng.randrange(6))}
            models[f'horizon:n{i}'] = model
        yield bundle(models)


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--prepare', action='store_true'); args = parser.parse_args()
    if args.prepare: lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-model-resolution-parity'; build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'; report_path.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaModelResolutionOracle.java'
    helper = oracle.with_name('VanillaCuboidModelOracle.java')
    crate = lab.ROOT / 'crates/horizon-assets'
    sources = [oracle, helper, Path(__file__), lab.ROOT / 'tools/test_cuboid_model_parity.py',
               lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'Cargo.toml',
               lab.ROOT / 'Cargo.lock', crate / 'Cargo.toml', *sorted(crate.rglob('*.rs'))]
    tested_sources = base.source_hashes(sources)
    with zipfile.ZipFile(jars['client']) as archive:
        names = sorted(name for name in archive.namelist() if name.startswith('assets/minecraft/models/block/') and name.endswith('.json'))
        models = {'minecraft:' + name.removeprefix('assets/minecraft/models/').removesuffix('.json'): archive.read(name).decode() for name in names}
    rows = [{'models': models, 'roots': list(models) + ['minecraft:builtin/missing', 'horizon:missing'],
             'queries': ['particle', '#particle', 'all', '#all', 'missingno', 'unknown', '', '#']}, *corpus()]
    inputs = build / 'inputs.jsonl'; inputs.write_text(''.join(json.dumps(row) + '\n' for row in rows)); fixtures = base.source_hashes([inputs])
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', dependencies, '-d', classes, oracle, helper], 'compile-java')
    run(['cargo', 'build', '--locked', '-p', 'horizon-assets', '--example', 'model_resolution_fixture'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'debug/examples' / ('model_resolution_fixture.exe' if os.name == 'nt' else 'model_resolution_fixture')
    run([binary, inputs, build / 'rust.jsonl'], 'rust')
    run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaModelResolutionOracle', inputs, build / 'client.jsonl'], 'client')
    rejected = decoding.canonicalize(build / 'client.jsonl'); decoding.canonicalize(build / 'rust.jsonl')
    matched = base.compare(build / 'client.jsonl', build / 'rust.jsonl')
    base.ensure_sources_unchanged(sources, tested_sources); base.ensure_sources_unchanged([inputs], fixtures)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'],
              'platform': platform.platform(), 'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'resource_models': len(names), 'graph_fixtures': len(rows), 'rejected_cyclic_roots': rejected,
              'sources_sha256': tested_sources, 'fixtures_sha256': fixtures, 'rust_output_sha256': lab.digest(build / 'rust.jsonl', 'sha256'),
              'references': [{'side': 'client', 'jar_sha256': lab.digest(jars['client'], 'sha256'),
                              'output_sha256': lab.digest(build / 'client.jsonl', 'sha256'), 'matched_output_lines': matched}],
              'scope': 'Actual ModelDiscovery and ResolvedModel/TextureSlots APIs over all bundled block models plus synthetic parent/alias graphs. Compares inherited geometry, AO/gui defaults, per-context display transforms, resolved materials, missing-model fallback, ignored built-in override, cyclic parent rejection, long alias/parent chains and slot lookup. Native rejection text/logging and discovery map iteration order are not compared. Geometry is shared through Arc in Rust; this does not prove quad baking, blockstate dispatch, pixels or gameplay.'}
    lab.write_json(report_path, report); print(f'Client model resolution: {matched:,} roots match ({rejected:,} cyclic); {report_path}', flush=True)


if __name__ == '__main__': main()
