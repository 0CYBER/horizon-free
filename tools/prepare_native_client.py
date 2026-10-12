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


WORLDGEN_BUNDLES = {
    'noises': 'worldgen/noise', 'densities': 'worldgen/density_function', 'rules': 'worldgen/material_rule',
    'conditions': 'worldgen/material_condition', 'settings': 'worldgen/noise_settings', 'carvers': 'worldgen/carver',
    'biomes': 'worldgen/biome', 'world_presets': 'worldgen/world_preset', 'features': 'worldgen/feature',
    'placed_features': 'worldgen/placed_feature', 'state_providers': 'worldgen/block_state_provider',
    'biome_tags': 'tags/worldgen/biome', 'feature_tags': 'tags/worldgen/feature', 'block_tags': 'tags/block',
    'fluid_tags': 'tags/fluid', 'processor_lists': 'worldgen/processor_list',
    'template_pools': 'worldgen/template_pool', 'structures': 'worldgen/structure',
    'structure_sets': 'worldgen/structure_set'}


def export_structure_blocks(target, jars, libraries):
    """Run the vanilla exporter of per-state rotate/mirror results and template flags."""
    lock = json.loads((lab.ROOT/'research-lock.json').read_text())
    source = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStructureBlockExporter.java'
    export = lab.LOCAL/'horizon-client-structures'
    classes = export/'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    with (export/'export.log').open('w') as log:
        subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-cp', dependencies,
                        '-d', str(classes), str(source)], check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        subprocess.run(['java', '-Xmx1G', '-cp', str(classes)+os.pathsep+dependencies,
                        'dev.horizonlab.parity.VanillaStructureBlockExporter', str(target)],
                       check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)


BLOCK_BEHAVIOR_FILES = ['block-properties.nbt', 'cached-collision.nbt', 'cached-support.nbt', 'survival-rules.json',
                        'survival-extra-definitions.json', 'placement-definitions.json']


def export_block_behavior(export, jars, libraries):
    """Run the vanilla exporters of block properties, support, collision and survival definitions.

    Feature placement needs them: without these files the generator skips every placed feature
    whose blocks are checked for survival or support (trees, flowers, grass, kelp, ...)."""
    try:
        from . import test_block_survival_parity as survival
    except ImportError:
        import test_block_survival_parity as survival
    lock = json.loads((lab.ROOT/'research-lock.json').read_text())
    tests = lab.ROOT/'mods/horizon/src/test/java'
    classes = export/'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    with (export/'export.log').open('w') as log:
        # The oracle's helper classes are compiled on demand from the source path.
        subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-cp', dependencies,
                        '-sourcepath', str(tests), '-d', str(classes),
                        str(tests/'dev/horizonlab/parity/VanillaSimpleBlockOracle.java')],
                       check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        subprocess.run(['java', '-Xmx1G', '-cp', str(classes)+os.pathsep+dependencies,
                        'dev.horizonlab.parity.VanillaSimpleBlockOracle', 'export', str(export)],
                       check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    rules = survival.bind_definitions(*[json.loads((export/name).read_text()) for name in
                                        ['survival-classes.json', 'survival-placement-classes.json',
                                         'survival-support-tags.json']])
    (export/'survival-rules.json').write_text(json.dumps(rules, separators=(',', ':')))


def prepare_worldgen(root, jars, local, libraries=None):
    """Write the chunk generator's resource bundle to root/worldgen (see GeneratorData::load)."""
    out = root / 'worldgen'
    out.mkdir(exist_ok=True)
    # Structure templates: gzip NBT files under structure/<namespace>/<path>.nbt.
    with zipfile.ZipFile(jars['server']) as server:
        prefix = 'data/minecraft/structure/'
        for name in server.namelist():
            if name.startswith(prefix) and name.endswith('.nbt'):
                relative = Path(name[len(prefix):])
                if relative.is_absolute() or '..' in relative.parts:
                    raise ValueError('Invalid structure path')
                target = out/'structure'/'minecraft'/relative
                data = server.read(name)
                if not target.exists() or target.read_bytes() != data:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(data)
    with zipfile.ZipFile(jars['server']) as server, zipfile.ZipFile(jars['client']) as client:
        for name, path in WORLDGEN_BUNDLES.items():
            prefix = f'data/minecraft/{path}/'
            def extract(archive):
                return {'minecraft:'+n[len(prefix):-5]: archive.read(n).decode('utf-8')
                        for n in sorted(archive.namelist()) if n.startswith(prefix) and n.endswith('.json')}
            resources = extract(server)
            if not resources or resources != extract(client):
                raise ValueError('Reference worldgen resources disagree: '+path)
            (out/(name+'.json')).write_text(json.dumps(resources, separators=(',', ':')), encoding='utf-8')
    facts = json.loads((root/'facts.json').read_text())
    (out/'block_flags.json').write_text(json.dumps([[s[1], s[2]] for s in facts['states']], separators=(',', ':')))
    reports = out/'biome_parameters'
    reports.mkdir(exist_ok=True)
    presets = reports
    for source in [root/'biome-parameters', local/'datagen/out/reports/biome_parameters/minecraft']:
        if (source/'overworld.json').exists():
            for report in sorted(source.glob('*.json')):
                (reports/report.name).write_bytes(report.read_bytes())
            break
    else:
        presets = None
    # Block behavior used by feature placement checks (survival, support, collision).
    # These come from the Java exporters of the simple-block parity run, or a direct export.
    behavior = local/'rust-simple-block-parity'
    blocks = out/'blocks'
    names = BLOCK_BEHAVIOR_FILES
    if libraries is not None and not all((behavior/name).exists() for name in names):
        behavior = local/'horizon-client-block-behavior'
        export_block_behavior(behavior, jars, libraries)
    if all((behavior/name).exists() for name in names):
        blocks.mkdir(exist_ok=True)
        for name in names:
            (blocks/name).write_bytes((behavior/name).read_bytes())
    if libraries is not None:
        blocks.mkdir(exist_ok=True)
        export_structure_blocks(blocks/'structure-blocks.json', jars, libraries)
    else:
        print('Feature block behavior exports missing; run tools/test_simple_block_parity.py for full decoration')
    if presets is None:
        # Fall back to the Java-exported quantized tables from the material-stage parity run.
        presets = local/'rust-material-stage-parity/presets.json'
        if not presets.exists():
            raise FileNotFoundError('No multi-noise biome parameter tables; run the data generator reports')
        (out/'presets.json').write_bytes(presets.read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true',
                        help='Import the pinned game files from your own copy first')
    lab.add_game_arguments(parser)
    args = parser.parse_args()
    if args.prepare:
        lab.prepare(args.minecraft_dir, args.server_jar)
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
    # Entity dimensions, categories and default attributes for the client's mobs.
    entity_types = root / 'entity-types.json'
    entity_types_oracle = oracle.with_name('VanillaEntityTypesExporter.java')
    entity_types_sources = base.source_hashes([entity_types_oracle])
    if not (previous.get('entity_types_exporters_sha256') == entity_types_sources and entity_types.exists()
            and previous.get('server_sha256') == origin['server_sha256']):
        classes = lab.LOCAL / 'entity-types' / 'classes'; classes.mkdir(parents=True, exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (root / 'entity-types-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(entity_types_oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaEntityTypesExporter', str(entity_types)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    # Block classes, instance configuration and per-state support/signal/fluid
    # facts for horizon-world's level behaviour (level::Behaviours::load).
    behaviours = root / 'block-behaviours.json'
    behaviour_oracle = oracle.with_name('VanillaBlockBehaviourExporter.java')
    behaviour_sources = base.source_hashes([behaviour_oracle])
    if not (previous.get('behaviour_exporters_sha256') == behaviour_sources and behaviours.exists()
            and previous.get('server_sha256') == origin['server_sha256']):
        classes = lab.LOCAL / 'block-behaviours' / 'classes'; classes.mkdir(parents=True, exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (root / 'behaviour-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(behaviour_oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaBlockBehaviourExporter', str(behaviours)],
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
    # Biome parameter tables come from the data generator reports above.
    prepare_worldgen(root, jars, lab.LOCAL, libraries)
    entity_models = root / 'entity-models.json'
    entity_oracle = oracle.with_name('VanillaEntityModelExporter.java')
    entity_sources = base.source_hashes([entity_oracle])
    if not (previous.get('entity_model_exporters_sha256') == entity_sources and entity_models.exists()
            and previous.get('client_sha256') == origin['client_sha256']):
        classes = lab.LOCAL / 'entity-models' / 'classes'; classes.mkdir(parents=True, exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
        with (root / 'entity-model-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(entity_oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaEntityModelExporter', str(entity_models)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    # Mob AI and animation facts: per-state path types, spawn placements and the
    # client's keyframe animation definitions (needs the client jar).
    mob_data = root / 'mob-data.json'
    mob_oracle = oracle.with_name('VanillaMobDataExporter.java')
    mob_sources = base.source_hashes([mob_oracle])
    if not (previous.get('mob_data_exporters_sha256') == mob_sources and mob_data.exists()
            and previous.get('client_sha256') == origin['client_sha256']):
        classes = lab.LOCAL / 'mob-data' / 'classes'; classes.mkdir(parents=True, exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
        with (root / 'mob-data-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(mob_oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaMobDataExporter', str(mob_data), str(jars['client'])],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    # Vanilla's built-in registries for in-game command parsing (mob effects,
    # data component types, dialogs, block entity and tag members).
    command_registries = root / 'command-registries.json'
    command_oracle = oracle.with_name('VanillaCommandParseOracle.java')
    command_sources = base.source_hashes([command_oracle])
    if not (previous.get('command_registries_exporters_sha256') == command_sources and command_registries.exists()
            and previous.get('server_sha256') == origin['server_sha256']):
        export = lab.LOCAL / 'command-registries'; classes = export / 'classes'; classes.mkdir(parents=True, exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (root / 'command-registries-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(command_oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaCommandParseOracle', 'registries', str(export)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        command_registries.write_bytes((export / 'registries.json').read_bytes())
    creative = root / 'creative-tabs.json'
    creative_oracle = oracle.with_name('VanillaCreativeTabsExporter.java')
    creative_sources = base.source_hashes([creative_oracle])
    if not (previous.get('creative_exporters_sha256') == creative_sources and creative.exists()
            and previous.get('server_sha256') == origin['server_sha256']):
        classes = lab.LOCAL / 'creative-tabs' / 'classes'; classes.mkdir(parents=True, exist_ok=True)
        dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
        with (root / 'creative-export.log').open('w') as log:
            subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                            '-cp', dependencies, '-d', str(classes), str(creative_oracle)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
            subprocess.run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
                            'dev.horizonlab.parity.VanillaCreativeTabsExporter', str(creative)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        recover_creative_tabs(creative, root)
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
                             'entity_model_exporters_sha256': entity_sources,
                             'creative_exporters_sha256': creative_sources,
                             'mob_data_exporters_sha256': mob_sources,
                             'entity_types_exporters_sha256': entity_types_sources,
                             'behaviour_exporters_sha256': behaviour_sources,
                             'command_registries_exporters_sha256': command_sources,
                             'model_states_sha256': lab.digest(model_states, 'sha256'), 'data_version': version['world_version'],
                             'facts_sha256': lab.digest(facts, 'sha256'),
                             'block_tags_sha256': lab.digest(root / 'block-tags.json', 'sha256'), 'resource_files': count})
    print(f'Prepared Horizon native-client resources: {root}')


def recover_creative_tabs(path, root):
    """Tabs whose generators need dynamic registries (banner patterns) fail in
    the bare bootstrap; recover their item order from the decompiled source."""
    import re
    tabs = json.loads(path.read_text())
    source = lab.LOCAL / 'decompiled/net/minecraft/world/item/CreativeModeTabs.java'
    if not source.exists():
        return
    items = json.loads((root / 'items.json').read_text())
    text = source.read_text()
    for tab in tabs:
        if tab['items'] or tab['type'] != 'CATEGORY':
            continue
        name = tab['id'].split(':')[1].upper()
        start = text.find(f'{name},')
        end = text.find('Registry.register(', start + 1)
        found = []
        for m in re.finditer(r'\b(Items|Blocks)\.([A-Z0-9_]+)(?:\.(\w+)\(\))?', text[start:end]):
            base, colour = m.group(2).lower(), m.group(3)
            item = 'minecraft:' + (f'{colour}_{base}' if colour and colour not in ('asItem', 'defaultBlockState', 'get') else base)
            if item in items and item not in found:
                found.append(item)
        tab['items'] = found
    path.write_text(json.dumps(tabs))


if __name__ == '__main__':
    main()
