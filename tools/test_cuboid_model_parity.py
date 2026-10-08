#!/usr/bin/env python3
"""Compare cuboid decoding with the actual pinned client parser (client-only API)."""
import argparse
import copy
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab, test_rust_parity as base


def corpus():
    element = {'from': [0, 0, 0], 'to': [16, 16, 16], 'faces': {'north': {'texture': '#all'}}}
    values = [None, False, True, 0, 1, -1, 15, 16, 90, 90.9, -90, 360, 450, 1e30,
              '', 'true', 'TRUE', 'false', 'north', 'NORTH', 'x', '#all', [], {}, [0, 0, 0]]
    for raw in ['null', '{}', '[]', 'false', '0', '"model"', '{"elements":[]}',
                '{"elements":[null]}', '{"elements":null}', '{"parent":"","textures":{}}',
                '{} trailing', '{} {}', '\ufeff{}', '', ' ', '{"unknown":NaN}',
                '{"unknown":Infinity}', '{"unknown":01}', '{"unknown":true,}', '{/*comment*/}',
                '{"unknown":"unescaped\nnewline"}', '{unquoted:0}']:
        yield raw
    for field in ['parent', 'ambientocclusion', 'gui_light', 'textures', 'display', 'elements']:
        for value in values:
            yield json.dumps({field: value})
    for name in ['block/cube_all', ':block/cube_all', 'horizon:test', ':', 'minecraft:',
                 'a:b:c', 'Minecraft:cube', 'a:b c', '../cube', 'é', 'a:/', 'a:.']:
        yield json.dumps({'parent': name})
        yield json.dumps({'textures': {'all': name}})
    for value in values + ['#', '##all', 'block/stone', ':block/stone',
                           {'sprite': 'block/glass'}, {'sprite': ''}, {'sprite': None}]:
        yield json.dumps({'textures': {'all': value}})
    for value in values:
        yield json.dumps({'textures': {'all': {'sprite': 'block/glass', 'force_translucent': value}}})
    for field in ['from', 'to', 'rotation', 'faces', 'shade_direction_override', 'light_emission', 'shade']:
        for value in values:
            modified = copy.deepcopy(element); modified[field] = value
            yield json.dumps({'elements': [modified]})
    for field in ['cullface', 'texture', 'tintindex', 'uv', 'rotation']:
        for value in values:
            modified = copy.deepcopy(element); modified['faces']['north'][field] = value
            yield json.dumps({'elements': [modified]})
    for direction in ['down', 'up', 'north', 'south', 'west', 'east', 'NORTH', 'north ', '']:
        for value in [None, {'texture': '#all'}, {}]:
            modified = copy.deepcopy(element); modified['faces'] = {direction: value}
            yield json.dumps({'elements': [modified]})
    for rotation in [{'origin': [8, 8, 8]}, {'origin': [8, 8, 8], 'x': 30},
                     {'origin': [8, 8, 8], 'y': 20, 'z': 10},
                     {'origin': [-32, 0, 64], 'axis': 'X', 'angle': 63, 'x': None},
                     {'origin': [0, 0, 0], 'angle': 45, 'x': 0},
                     {'origin': [0, 0, 0], 'axis': 'y', 'x': 0}]:
        modified = copy.deepcopy(element); modified['rotation'] = rotation
        yield json.dumps({'elements': [modified]})
    for key in ['origin', 'axis', 'angle', 'rescale', 'x', 'y', 'z']:
        for value in values:
            rotation = {'origin': [8, 8, 8], 'axis': 'y', 'angle': 22.5} if key in ['axis', 'angle'] else {'origin': [8, 8, 8], 'x': 45}
            rotation[key] = value
            modified = copy.deepcopy(element); modified['rotation'] = rotation
            yield json.dumps({'elements': [modified]})
    for context in ['thirdperson_lefthand', 'thirdperson_righthand', 'firstperson_lefthand',
                    'firstperson_righthand', 'head', 'gui', 'ground', 'fixed', 'on_shelf', 'unknown']:
        for value in values + [{'rotation': [1, 2, 3], 'translation': [-81, -0.0, 81], 'scale': [-5, 0, 5]}]:
            yield json.dumps({'display': {context: value}})
    for left in ['thirdperson_lefthand', 'firstperson_lefthand']:
        right = left.replace('lefthand', 'righthand')
        for value in [None, {}, {'scale': [1, 2, 3]}]:
            yield json.dumps({'display': {left: value, right: {'rotation': [1, 2, 3]}}})
        yield json.dumps({'display': {right: {'rotation': [1, 2, 3]}}})
    # Raw tokens retain decimal spelling, integer wraparound and direct f32 rounding.
    numbers = ['-0.0', '1e02', '90.999', '-90.999', '4294967296', '4294967297',
               '9223372036854775809', '-9223372036854775809', '1e100', '1e-100',
               '1e2147483647', '1e2147483648', '1e-2147483648',
               '1.00000005960464477539062500000001', '1e-46', '-1e-46', '3.5e38',
               '1e9999', '1e10000', '1e-9999', '1e-10000', '1.0e10000', '1.0e10001',
               '1' * 1023, '1' * 1024, '1' * 10000, '1' * 10001,
               *('1' + '0' * (n - 2) + '1' for n in [20, 64, 65, 66, 100, 1023, 1024]),
               '1844674407370955161601', '-1844674407370955161601',
               '18446744073709551616.1', '18446744073709551616e-18']
    for number in numbers:
        for field in ['tintindex', 'rotation', 'texture']:
            yield '{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all","' + field + '":' + number + '}}}]}'
        yield '{"display":{"gui":{"rotation":[' + number + ',0,0],"translation":[' + number + ',0,0],"scale":[' + number + ',0,0]}}}'
    # Strict stream parsing rejects scanner failures even in ignored fields;
    # quoted copies remain ordinary strings.
    for number in numbers:
        yield '{"unknown":' + number + '}'
        yield '{"unknown":"' + number + '"}'
    rng = random.Random(0x4355424f4944)
    for _ in range(1200):
        modified = copy.deepcopy(element)
        modified['from'] = [rng.uniform(-17, 33) for _ in range(3)]
        modified['to'] = [rng.uniform(-17, 33) for _ in range(3)]
        modified['rotation'] = {'origin': [rng.uniform(-100, 100) for _ in range(3)], 'rescale': rng.choice([False, True])}
        if rng.randrange(2):
            modified['rotation'].update(axis=rng.choice(['x', 'y', 'z']), angle=rng.uniform(-720, 720))
        else:
            modified['rotation'].update(zip(['x', 'y', 'z'], [rng.uniform(-720, 720) for _ in range(3)]))
        modified['faces'] = {rng.choice(['down', 'up', 'north', 'south', 'west', 'east']): {
            'texture': '#all', 'rotation': 90 * rng.randrange(-12, 13), 'tintindex': rng.randrange(-4, 5),
            'uv': [rng.uniform(-32, 32) for _ in range(4)]}}
        modified['light_emission'] = rng.randrange(16)
        yield json.dumps({'elements': [modified]})


def canonicalize(path):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    path.write_text(''.join(json.dumps(row, sort_keys=True, separators=(',', ':')) + '\n' for row in rows))
    return sum('error' in row for row in rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-cuboid-model-parity'; build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'; report_path.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCuboidModelOracle.java'
    crate = lab.ROOT / 'crates/horizon-assets'
    sources = [oracle, Path(__file__), lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/test_rust_parity.py',
               lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock', crate / 'Cargo.toml',
               *sorted(crate.rglob('*.rs'))]
    tested_sources = base.source_hashes(sources)
    with zipfile.ZipFile(jars['client']) as archive:
        names = sorted(name for name in archive.namelist() if name.startswith('assets/minecraft/models/block/') and name.endswith('.json'))
        rows = [(name, archive.read(name).decode()) for name in names]
    synthetic = list(corpus())
    rows += [(f'synthetic/{index}', raw) for index, raw in enumerate(synthetic)]
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    fixtures = base.source_hashes([inputs])
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', dependencies, '-d', classes, oracle], 'compile-java')
    run(['cargo', 'build', '--locked', '-p', 'horizon-assets', '--example', 'cuboid_model_fixture'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'debug/examples' / ('cuboid_model_fixture.exe' if os.name == 'nt' else 'cuboid_model_fixture')
    run([binary, inputs, build / 'rust.jsonl'], 'rust')
    run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaCuboidModelOracle', inputs, build / 'client.jsonl'], 'client')
    rejected = canonicalize(build / 'client.jsonl'); canonicalize(build / 'rust.jsonl')
    matched = base.compare(build / 'client.jsonl', build / 'rust.jsonl')
    base.ensure_sources_unchanged(sources, tested_sources); base.ensure_sources_unchanged([inputs], fixtures)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'],
              'platform': platform.platform(), 'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'resource_models': len(names), 'synthetic_models': len(synthetic), 'rejected_models': rejected,
              'sources_sha256': tested_sources, 'fixtures_sha256': fixtures,
              'rust_output_sha256': lab.digest(build / 'rust.jsonl', 'sha256'),
              'references': [{'side': 'client', 'jar_sha256': lab.digest(jars['client'], 'sha256'),
                              'output_sha256': lab.digest(build / 'client.jsonl', 'sha256'), 'matched_output_lines': matched}],
              'scope': 'Actual pinned CuboidModel.fromStream parser compared over every bundled block model and synthetic JSON/numeric/syntax cases. Decoded geometry, texture materials/references, nullable children, inheritance markers, face quadrants, light emission and display transforms; floats compare by IEEE-754 bits. Rejections normalize exception text. This client-only API is absent from the server. Does not yet validate parent resolution, rotation matrices, quad baking, blockstate dispatch, rendered pixels or gameplay.'}
    lab.write_json(report_path, report)
    print(f'Client cuboid decoding: {matched:,} models match ({rejected:,} rejected); {report_path}', flush=True)


if __name__ == '__main__':
    main()
