#!/usr/bin/env python3
"""Prepare private pinned resources for Horizon's native development client."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen, test_model_baking_parity as baking
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_model_baking_parity as baking


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true', help='Fetch pinned reference artifacts first')
    args = parser.parse_args()
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    root = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    root.mkdir(parents=True, exist_ok=True)
    facts = root / 'facts.json'
    manifest = root / 'horizon-resources.json'
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java'
    origin = {'minecraft': lock['minecraft']['version'],
              'client_sha256': lab.digest(jars['client'], 'sha256'),
              'server_sha256': lab.digest(jars['server'], 'sha256'),
              'exporter_sha256': lab.digest(oracle, 'sha256')}
    previous = json.loads(manifest.read_text()) if manifest.exists() else {}
    if not (all(previous.get(k) == v for k, v in origin.items()) and facts.exists()
            and previous.get('facts_sha256') == lab.digest(facts, 'sha256')):
        classes = root / 'classes'
        classes.mkdir(exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (root / 'export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(oracle)], check=True,
                           stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaHeightmapOracle', 'export', str(facts)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    binding = oracle.with_name('VanillaBlockStateBindingOracle.java')
    helper = oracle.with_name('VanillaCuboidModelOracle.java')
    binding_sources = base.source_hashes([binding, helper])
    model_states = root / 'model-states.json'
    if not (previous.get('model_state_exporters_sha256') == binding_sources
            and previous.get('client_sha256') == origin['client_sha256']
            and model_states.exists()
            and previous.get('model_states_sha256') == lab.digest(model_states, 'sha256')):
        classes = root / 'client-classes'; classes.mkdir(exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
        with (root / 'model-state-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(binding), str(helper)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaBlockStateBindingOracle', 'export', str(model_states)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    tags = worldgen.tag_resources(jars['server'])
    if tags != worldgen.tag_resources(jars['client']):
        raise ValueError('Reference block tags disagree')
    lab.write_json(root / 'block-tags.json', tags)
    visibility = oracle.with_name('VanillaFaceVisibilityOracle.java')
    collision = oracle.with_name('VanillaCollisionOracle.java')
    collision_sources = base.source_hashes([collision])
    collision_file = root / 'cached-collision.nbt'
    if not (previous.get('collision_exporters_sha256') == collision_sources
            and previous.get('server_sha256') == origin['server_sha256']
            and collision_file.exists()
            and previous.get('collision_sha256') == lab.digest(collision_file, 'sha256')):
        export = lab.LOCAL/'horizon-client-collision'; export.mkdir(exist_ok=True)
        classes = export/'classes'; classes.mkdir(exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (export/'export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(collision)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx768M', '-cp', str(classes)+os.pathsep+dependencies,
                            'dev.horizonlab.parity.VanillaCollisionOracle', 'generate', str(export)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        collision_file.write_bytes((export/'cached-collision.nbt').read_bytes())
    render_sources = base.source_hashes([visibility, collision, oracle])
    render_files = ['render-definitions.json', 'occlusion.nbt']
    tag_hash = lab.digest(root/'block-tags.json', 'sha256')
    if not (previous.get('render_exporters_sha256') == render_sources
            and previous.get('server_sha256') == origin['server_sha256']
            and previous.get('block_tags_sha256') == tag_hash
            and all((root/name).exists() and previous.get('render_files_sha256', {}).get(name)
                    == lab.digest(root/name, 'sha256') for name in render_files)):
        export = lab.LOCAL/'horizon-client-render'; export.mkdir(exist_ok=True)
        lab.write_json(export/'tags.json', tags)
        classes = export/'classes'; classes.mkdir(exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (export/'export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(visibility), str(collision), str(oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx768M', '-cp', str(classes)+os.pathsep+dependencies,
                            'dev.horizonlab.parity.VanillaFaceVisibilityOracle', 'export', str(export)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        (root/'render-definitions.json').write_bytes((export/'definitions.json').read_bytes())
        (root/'occlusion.nbt').write_bytes((export/'occlusion.nbt').read_bytes())
    count = 0
    prefixes = ('assets/minecraft/blockstates/', 'assets/minecraft/models/block/',
                'assets/minecraft/textures/block/')
    with zipfile.ZipFile(jars['client']) as archive:
        version = json.loads(archive.read('version.json'))
        for name in archive.namelist():
            if not name.startswith(prefixes) or name.endswith('/'):
                continue
            relative = Path(name)
            if relative.is_absolute() or '..' in relative.parts:
                raise ValueError('Invalid resource path')
            target = root / relative
            data = archive.read(name)
            if not target.exists() or target.read_bytes() != data:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            count += 1
    lab.write_json(manifest, {'schema': 4, **origin,
                             'collision_exporters_sha256': collision_sources,
                             'collision_sha256': lab.digest(collision_file, 'sha256'),
                             'render_exporters_sha256': render_sources,
                             'render_files_sha256': {name: lab.digest(root/name, 'sha256') for name in render_files},
                             'model_state_exporters_sha256': binding_sources,
                             'model_states_sha256': lab.digest(model_states, 'sha256'), 'data_version': version['world_version'],
                             'facts_sha256': lab.digest(facts, 'sha256'),
                             'block_tags_sha256': lab.digest(root / 'block-tags.json', 'sha256'), 'resource_files': count})
    print(f'Prepared Horizon native-client resources: {root}')


if __name__ == '__main__':
    main()
