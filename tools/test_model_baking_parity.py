#!/usr/bin/env python3
"""Compare cuboid quad positions, UVs and element matrices with the pinned client."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_cuboid_model_parity as decoding, test_model_resolution_parity as resolution
except ImportError:
    import lab, test_rust_parity as base, test_cuboid_model_parity as decoding, test_model_resolution_parity as resolution


def corpus():
    yield from resolution.corpus()
    directions = ['down', 'up', 'north', 'south', 'west', 'east']
    models = {}
    def add(rotation, start=None, end=None, explicit=False):
        index = len(models)
        faces = {d: {'texture': '#all', 'cullface': d, 'rotation': 90 * (index % 4),
                     'tintindex': index % 3 - 1} for d in directions}
        if explicit:
            for face in faces.values(): face['uv'] = [-0.0, 3.25, 15.5, -1.0]
        element = {'from': start or [0, 0, 0], 'to': end or [16, 16, 16], 'faces': faces,
                   'light_emission': index % 16}
        if rotation is not None: element['rotation'] = rotation
        if index % 3 == 0: element['shade_direction_override'] = directions[index % 6]
        models[f'horizon:geometry_{index}'] = {'textures': {'all': 'block/stone'}, 'elements': [element]}
    for bounds in [([0,0,0],[16,16,16]), ([16,16,16],[0,0,0]), ([0,8,0],[16,8,16]),
                   ([0,0,0],[0,0,0]), ([-16,-16,-16],[32,32,32]), ([-0.0,0,0],[16,16,16])]:
        for explicit in [False, True]: add(None, *bounds, explicit)
    for angle in [-720,-360,-180,-90,-63,-45,-22.5,-0.0,0,22.5,45,63,90,180,270,360,540,720]:
        for axis in ['x','y','z']:
            for rescale in [False,True]:
                add({'origin':[8,8,8], 'axis':axis, 'angle':angle, 'rescale':rescale}, explicit=True)
    for x in [0,22.5,90,180]:
        for y in [0,45,90,270]:
            for z in [-90,0,45,180]:
                for rescale in [False,True]:
                    add({'origin':[8,3,-4], 'x':x, 'y':y, 'z':z, 'rescale':rescale})
    rng=random.Random(0x42414b494e47)
    for _ in range(1200):
        rotation={'origin':[rng.uniform(-64,64) for _ in range(3)], 'rescale':bool(rng.randrange(2))}
        if rng.randrange(2): rotation.update(axis=rng.choice(['x','y','z']), angle=rng.uniform(-3600,3600))
        else: rotation.update(zip(['x','y','z'],[rng.uniform(-3600,3600) for _ in range(3)]))
        add(rotation, [rng.uniform(-16,32) for _ in range(3)], [rng.uniform(-16,32) for _ in range(3)], bool(rng.randrange(2)))
    row=resolution.bundle(models)
    for token in ['1e100','-1e100','1e-100','-1e-100']:
        name='horizon:extreme_'+str(len(row['models']))
        raw=json.dumps({'elements':[{'from':[0,0,0],'to':[16,16,16],'faces':{'up':{'texture':'#all'}},'rotation':{'origin':[8,8,8],'axis':'x','angle':'TOKEN','rescale':True}}]})
        row['models'][name]=raw.replace('"TOKEN"',token); row['roots'].append(name)
    yield row


def client_libraries():
    _, directory=lab.config()
    manifest=json.loads((directory/'manifest.json').read_text())
    libraries=[]
    for artifact in lab.client_library_artifacts(manifest):
        relative=Path(artifact['path'])
        if relative.is_absolute() or '..' in relative.parts: raise ValueError('Unsafe client library path')
        path=directory/'libraries/client'/relative
        if lab.digest(path,'sha1')!=artifact['sha1']: raise ValueError('Client dependency checksum changed: '+str(relative))
        libraries.append(path)
    return libraries


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--prepare', action='store_true'); args = parser.parse_args()
    if args.prepare: lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-model-baking-parity'; build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'; report_path.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaModelBakingOracle.java'
    helper = oracle.with_name('VanillaCuboidModelOracle.java')
    crate = lab.ROOT / 'crates/horizon-assets'
    sources = [oracle, helper, Path(__file__), lab.ROOT / 'tools/test_cuboid_model_parity.py', lab.ROOT / 'tools/test_model_resolution_parity.py',
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
    dependencies = os.pathsep.join(map(str, [jars['client'], *client_libraries()]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', dependencies, '-d', classes, oracle, helper], 'compile-java')
    run(['cargo', 'build', '--locked', '-p', 'horizon-assets', '--example', 'model_baking_fixture'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'debug/examples' / ('model_baking_fixture.exe' if os.name == 'nt' else 'model_baking_fixture')
    run([binary, inputs, build / 'rust.jsonl'], 'rust')
    run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaModelBakingOracle', inputs, build / 'client.jsonl'], 'client')
    rejected = decoding.canonicalize(build / 'client.jsonl'); decoding.canonicalize(build / 'rust.jsonl')
    matched = base.compare(build / 'client.jsonl', build / 'rust.jsonl')
    base.ensure_sources_unchanged(sources, tested_sources); base.ensure_sources_unchanged([inputs], fixtures)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'],
              'platform': platform.platform(), 'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'resource_models': len(names), 'graph_fixtures': len(rows), 'rejected_roots': rejected,
              'sources_sha256': tested_sources, 'fixtures_sha256': fixtures, 'rust_output_sha256': lab.digest(build / 'rust.jsonl', 'sha256'),
              'references': [{'side': 'client', 'jar_sha256': lab.digest(jars['client'], 'sha256'),
                              'output_sha256': lab.digest(build / 'client.jsonl', 'sha256'), 'matched_output_lines': matched}],
              'scope': 'Actual client FaceBakery geometry overload with a real unit-atlas sprite and default JOML math options. Identity blockstate transform, all bundled cuboid geometry and synthetic rotations, rescaling, default/explicit UVs, winding, degenerate and reversed bounds, inherited models and missing materials. Positions, UVs and element matrix coefficients compare by float bits. This does not cover blockstate transforms/UV lock, atlas packing, transparency classification, lighting, GPU rendering or gameplay.'}
    lab.write_json(report_path, report); print(f'Client cuboid baking: {matched:,} roots match ({rejected:,} rejected); {report_path}', flush=True)


if __name__ == '__main__': main()
