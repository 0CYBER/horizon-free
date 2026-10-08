#!/usr/bin/env python3
"""Compare blockstate-transformed cuboid baking against the pinned Java client."""
import itertools
import json
import os
from pathlib import Path
import platform
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking, test_cuboid_model_parity as decoding
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking, test_cuboid_model_parity as decoding


def corpus(archive):
    models = {'minecraft:' + name.removeprefix('assets/minecraft/models/').removesuffix('.json'): archive.read(name).decode()
              for name in sorted(archive.namelist()) if name.startswith('assets/minecraft/models/block/') and name.endswith('.json')}
    groups = {}
    definitions = 0
    for name in sorted(archive.namelist()):
        if not name.startswith('assets/minecraft/blockstates/') or not name.endswith('.json'):
            continue
        definitions += 1
        root = json.loads(archive.read(name))
        for choice in [*root.get('variants', {}).values(), *(s['apply'] for s in root.get('multipart', []))]:
            for variant in choice if isinstance(choice, list) else [choice]:
                state = tuple(variant.get(axis, 0) % 360 // 90 for axis in ['x', 'y', 'z']) + (variant.get('uvlock', False),)
                model = variant['model']
                if ':' not in model:
                    model = 'minecraft:' + model
                groups.setdefault(state, set()).add(model)
    rows = [{'models': models, 'roots': sorted(roots), 'transform': state} for state, roots in sorted(groups.items())]
    resource_pairs = sum(len(roots) for roots in groups.values())
    # All 128 transform/lock combinations on asymmetric, reversed and degenerate
    # geometry, explicit/default UVs, signed zero and representative element rotations.
    synthetic = list(baking.corpus())[-1]['models']
    names = list(synthetic)
    selected = names[:12] + names[12:248:9] + names[248:504:13] + names[504::53] + names[-4:]
    focused = {name: synthetic[name] for name in dict.fromkeys(selected)}
    for turns in itertools.product(range(4), repeat=3):
        for lock in [False, True]:
            rows.append({'models': focused, 'roots': list(focused), 'transform': [*turns, lock]})
    # Cover every remaining arbitrary-angle element fixture under a deterministic
    # transform; do not multiply the whole large corpus by 128 unnecessarily.
    for state in itertools.product(range(4), range(4), [False, True]):
        x, y, lock = state
        subset = {name: raw for i, (name, raw) in enumerate(synthetic.items()) if i % 32 == x*8+y*2+lock}
        rows.append({'models': subset, 'roots': list(subset), 'transform': [x, y, (x+y)%4, lock]})
    return rows, definitions, resource_pairs, len(focused)


def main():
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'rust-blockstate-transform-parity'; build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'; report_path.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaModelBakingOracle.java'
    helper = oracle.with_name('VanillaCuboidModelOracle.java'); crate = lab.ROOT / 'crates/horizon-assets'
    sources = [oracle, helper, Path(__file__), lab.ROOT/'tools/test_model_baking_parity.py',
               lab.ROOT/'tools/test_cuboid_model_parity.py', lab.ROOT/'tools/test_model_resolution_parity.py',
               lab.ROOT/'tools/lab.py', lab.ROOT/'tools/test_rust_parity.py', lab.ROOT/'Cargo.toml',
               lab.ROOT/'Cargo.lock', crate/'Cargo.toml', *sorted(crate.rglob('*.rs'))]
    tested = base.source_hashes(sources)
    with zipfile.ZipFile(jars['client']) as archive:
        rows, definitions, resource_pairs, focused = corpus(archive)
    inputs = build/'inputs.jsonl'; inputs.write_text(''.join(json.dumps(row)+'\n' for row in rows))
    fixtures = base.source_hashes([inputs]); classes = build/'classes'; classes.mkdir(exist_ok=True)
    def run(command, name):
        with (build/(name+'.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', dependencies, '-d', classes, oracle, helper], 'compile-java')
    run(['cargo', 'build', '--locked', '-p', 'horizon-assets', '--example', 'model_baking_fixture'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory'])/'debug/examples'/('model_baking_fixture.exe' if os.name == 'nt' else 'model_baking_fixture')
    run([binary, inputs, build/'rust.jsonl'], 'rust')
    run(['java', '-Xmx768M', '-cp', str(classes)+os.pathsep+dependencies, 'dev.horizonlab.parity.VanillaModelBakingOracle', inputs, build/'client.jsonl'], 'client')
    rejected = decoding.canonicalize(build/'client.jsonl'); decoding.canonicalize(build/'rust.jsonl')
    matched = base.compare(build/'client.jsonl', build/'rust.jsonl')
    base.ensure_sources_unchanged(sources, tested); base.ensure_sources_unchanged([inputs], fixtures)
    quads = sum(len(json.loads(line).get('baked', {}).get('quads', [])) for line in (build/'client.jsonl').open())
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'],
              'platform': platform.platform(), 'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(), 'resource_definitions': definitions,
              'resource_model_transform_pairs': resource_pairs, 'synthetic_models_per_all_128_transforms': focused,
              'graph_fixtures': len(rows), 'rejected_roots': rejected, 'baked_quads': quads,
              'sources_sha256': tested, 'fixtures_sha256': fixtures, 'rust_output_sha256': lab.digest(build/'rust.jsonl', 'sha256'),
              'references': [{'side': 'client', 'jar_sha256': lab.digest(jars['client'], 'sha256'),
                              'output_sha256': lab.digest(build/'client.jsonl', 'sha256'), 'matched_output_lines': matched}],
              'scope': 'Actual FaceBakery with Variant.SimpleModelState and unit-atlas sprite, default JOML math. Every distinct model/XYZ-rotation/UV-lock tuple in bundled blockstates; all 64 quarter-turn combinations with and without UV lock on synthetic geometry. Exact float bits for positions, UVs, element matrices, orientation and all six inverse UV matrices; native Direction.rotate for culling. Includes identity UV lock, degenerate/reversed geometry, element rotations, signed zero and float overflow. Does not establish atlas packing, shader pixels, lighting, render classification, renderer integration or gameplay parity.'}
    lab.write_json(report_path, report)
    print(f'Native transformed baking: {matched:,} roots / {quads:,} quads match ({rejected:,} rejected); {report_path}', flush=True)


if __name__ == '__main__':
    main()
