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
    movement = oracle.with_name('VanillaMovementRegistryOracle.java')
    attributes = oracle.with_name('VanillaEntityAttributeOracle.java')
    landing = oracle.with_name('VanillaBlockLandingOracle.java')
    motion_sources = base.source_hashes([movement, attributes, oracle, landing, collision])
    motion_files = ['movement-properties.nbt', 'attributes.json', 'landing-definitions.json']
    if not (previous.get('motion_exporters_sha256') == motion_sources
            and previous.get('server_sha256') == origin['server_sha256']
            and previous.get('block_tags_sha256') == lab.digest(root/'block-tags.json', 'sha256')
            and all((root/name).exists() and previous.get('motion_files_sha256', {}).get(name)
                    == lab.digest(root/name, 'sha256') for name in motion_files)):
        export = lab.LOCAL/'horizon-client-motion'; export.mkdir(exist_ok=True)
        lab.write_json(export/'tags.json', tags)
        classes = export/'classes'; classes.mkdir(exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (export/'export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(movement), str(attributes), str(oracle), str(landing), str(collision)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            for name in ['VanillaBlockLandingOracle', 'VanillaEntityAttributeOracle']:
                subprocess.run(['java', '-Xmx768M', '-cp', str(classes)+os.pathsep+dependencies,
                                'dev.horizonlab.parity.'+name, 'export', str(export)],
                               check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        (root/'movement-properties.nbt').write_bytes((export/'movement-properties.nbt').read_bytes())
        (root/'attributes.json').write_bytes((export/'catalog.json').read_bytes())
        (root/'landing-definitions.json').write_bytes((export/'landing-definitions.json').read_bytes())
    damage = oracle.with_name('VanillaDamageSourceOracle.java')
    health = oracle.with_name('VanillaPlayerHealthOracle.java')
    health_sources = base.source_hashes([damage, health, attributes])
    health_files = ['damage-types.json', 'damage-tags.json', 'damage-catalog.json', 'player-float-ids.json']
    if not (previous.get('health_exporters_sha256') == health_sources
            and previous.get('server_sha256') == origin['server_sha256']
            and all((root/name).exists() and previous.get('health_files_sha256', {}).get(name)
                    == lab.digest(root/name, 'sha256') for name in health_files)):
        export = lab.LOCAL/'horizon-client-health'; export.mkdir(exist_ok=True)
        with zipfile.ZipFile(jars['server']) as archive, zipfile.ZipFile(jars['client']) as client_archive:
            for file, prefix in [('types.json', 'data/minecraft/damage_type/'), ('tags.json', 'data/minecraft/tags/damage_type/')]:
                def extract(source):
                    return {'minecraft:'+n[len(prefix):-5]: source.read(n).decode('utf-8')
                            for n in sorted(source.namelist()) if n.startswith(prefix) and n.endswith('.json')}
                resources = extract(archive)
                if resources != extract(client_archive): raise ValueError('Reference damage resources disagree')
                lab.write_json(export/file, resources)
        classes = export/'classes'; classes.mkdir(exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (export/'export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(damage), str(health), str(attributes)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            for name in ['VanillaPlayerHealthOracle', 'VanillaDamageSourceOracle']:
                subprocess.run(['java', '-Xmx768M', '-cp', str(classes)+os.pathsep+dependencies,
                                'dev.horizonlab.parity.'+name, 'export', str(export)],
                               check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        for source, target in [('types.json','damage-types.json'), ('tags.json','damage-tags.json'),
                               ('catalog.json','damage-catalog.json'), ('ids.json','player-float-ids.json')]:
            (root/target).write_bytes((export/source).read_bytes())
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
    # The playable client needs every client asset (textures, models, fonts, GUI)
    # and every data-pack resource (worldgen, recipes, loot tables, tags).
    prefixes = ('assets/', 'data/')
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
    gameplay = root / 'gameplay-facts.json'
    gameplay_oracle = oracle.with_name('VanillaGameplayFactsExporter.java')
    gameplay_sources = base.source_hashes([gameplay_oracle])
    if not (previous.get('gameplay_exporters_sha256') == gameplay_sources and gameplay.exists()
            and previous.get('server_sha256') == origin['server_sha256']):
        classes = lab.LOCAL / 'gameplay-facts' / 'classes'; classes.mkdir(parents=True, exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (root / 'gameplay-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(gameplay_oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaGameplayFactsExporter', str(gameplay)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    items = root / 'items.json'
    if not items.exists() or previous.get('server_sha256') != origin['server_sha256']:
        # Vanilla's own data generator reports every item's default components.
        datagen = lab.LOCAL / 'datagen'; datagen.mkdir(exist_ok=True)
        with (datagen / 'datagen.log').open('w') as log:
            subprocess.run(['java', '-DbundlerMainClass=net.minecraft.data.Main', '-jar',
                            str(jars['server'].with_name('server.jar')), '--reports', '--output', 'out'],
                           cwd=datagen, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        reports = datagen / 'out' / 'reports'
        components = {f'minecraft:{f.stem}': json.loads(f.read_text())['components']
                      for f in sorted((reports / 'minecraft' / 'components' / 'item').glob('*.json'))}
        items.write_text(json.dumps(components))
        (root / 'biome-parameters').mkdir(exist_ok=True)
        for f in (reports / 'biome_parameters' / 'minecraft').glob('*.json'):
            (root / 'biome-parameters' / f.name).write_bytes(f.read_bytes())
    lab.write_json(manifest, {'schema': 7, **origin,
                             'health_exporters_sha256': health_sources,
                             'health_files_sha256': {name: lab.digest(root/name, 'sha256') for name in health_files},
                             'motion_exporters_sha256': motion_sources,
                             'motion_files_sha256': {name: lab.digest(root/name, 'sha256') for name in motion_files},
                             'collision_exporters_sha256': collision_sources,
                             'collision_sha256': lab.digest(collision_file, 'sha256'),
                             'render_exporters_sha256': render_sources,
                             'render_files_sha256': {name: lab.digest(root/name, 'sha256') for name in render_files},
                             'model_state_exporters_sha256': binding_sources,
                             'gameplay_exporters_sha256': gameplay_sources,
                             'model_states_sha256': lab.digest(model_states, 'sha256'), 'data_version': version['world_version'],
                             'facts_sha256': lab.digest(facts, 'sha256'),
                             'block_tags_sha256': lab.digest(root / 'block-tags.json', 'sha256'), 'resource_files': count})
    print(f'Prepared Horizon native-client resources: {root}')


if __name__ == '__main__':
    main()
