#!/usr/bin/env python3
"""Compare initial block survival rules with pinned vanilla."""
import argparse
import json
import itertools
import random
import struct
import os
from pathlib import Path
import platform
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


CLASS_RULES={
    'FireBlock':'Fire',
    'CactusBlock':'Cactus',
    'PathBlock':'Path',
    'MushroomBlock':'Mushroom',

    'MultifaceBlock':'Multiface',
    'VineBlock':'Vine',
    'HangingMossBlock':'HangingMoss',
    'SeaPickleBlock':'SeaPickle',
    'SnowLayerBlock':'SnowLayer',

    'BellBlock':'Bell','ShelfMushroomBlock':'FullBehind',
    'GrowingPlantBlock':'GrowingPlant',
    'MangrovePropaguleBlock':'MangrovePropagule',
    'SpeleothemBlock':'Speleothem',
    'ScaffoldingBlock':'Scaffolding',
    'PistonHeadBlock':'PistonHead',
    'ChorusPlantBlock':'ChorusPlant',
    'ChorusFlowerBlock':'ChorusFlower',
    'PitcherCropBlock':'PitcherCrop',

    'BigDripleafBlock':'BigDripleaf','BigDripleafStemBlock':'BigDripleafStem','FarmlandBlock':'Farmland','CocoaBlock':'Cocoa',
    'SugarCaneBlock':'SugarCane','SporeBlossomBlock':'SporeBlossom','FrogspawnBlock':'Frogspawn','SmallDripleafBlock':'SmallDripleaf','TallSeagrassBlock':'TallSeagrass',
    'DoorBlock':'Door',
    'BaseRailBlock':'RigidBelow','LeafLitterBlock':'FullBelow','WallSignBlock':'SolidBehind','GrindstoneBlock':'Always','RedstoneWallTorchBlock':'FullBehind','BubbleColumnBlock':'BubbleColumn','RedstoneWireBlock':'RedstoneWire','SoulFireBlock':'TagBelow:minecraft:soul_fire_base_blocks',
    'BlockBehaviour':'Always','CakeBlock':'SolidBelow','CandleCakeBlock':'SolidBelow','BannerBlock':'SolidBelow','StandingSignBlock':'SolidBelow',
    'WallBannerBlock':'SolidBehind','CarpetBlock':'NonAirBelow','MossyCarpetBlock':'MossCarpet','BaseTorchBlock':'CenterBelow','CandleBlock':'CenterBelow','LanternBlock':'Lantern',
    'HangingRootsBlock':'FullAbove','LadderBlock':'FullBehind','AmethystClusterBlock':'FullBehind','TripWireHookBlock':'FullBehind','BaseCoralWallFanBlock':'FullBehind','WallTorchBlock':'FullBehind',
    'BasePressurePlateBlock':'RigidOrCenterBelow','DiodeBlock':'RigidBelow','BaseCoralPlantTypeBlock':'FullBelow','CeilingHangingSignBlock':'CenterAbove',
    'FaceAttachedHorizontalDirectionalBlock':'FaceAttached','BambooSaplingBlock':'SupportsBamboo','BambooStalkBlock':'SupportsBamboo',
}

VEGETATION_RULES={
    'LilyPadBlock':'LilyPad',
    'VegetationBlock':'TagBelow:minecraft:supports_vegetation',
    'AzaleaBlock':'TagBelow:minecraft:supports_azalea',
    'DryVegetationBlock':'TagBelow:minecraft:supports_dry_vegetation',
    'NetherSproutsBlock':'TagBelow:minecraft:supports_nether_sprouts',
    'NetherWartBlock':'TagBelow:minecraft:supports_nether_wart',
    'WitherRoseBlock':'TagBelow:minecraft:supports_wither_rose',
    'CactusFlowerBlock':'CactusFlower','SeagrassBlock':'Seagrass',
}

def bind_definitions(classes,placement,support_tags):
    definitions={name:CLASS_RULES[owner] for name,owner in classes.items() if owner in CLASS_RULES}
    for name,owner in classes.items():
        if owner=='CropBlock' and placement.get(name)=='CropBlock':definitions[name]='Crop'
        if owner=='DoublePlantBlock' and placement.get(name)=='VegetationBlock':definitions[name]='DoublePlant'
        if owner!='VegetationBlock':continue
        method=placement[name]
        if method in VEGETATION_RULES:definitions[name]=VEGETATION_RULES[method]
        elif method in ['AttachedStemBlock','StemBlock','NetherFungusBlock','NetherRootsBlock']:
            field='stemSupportBlocks' if method=='StemBlock' else 'supportBlocks'
            definitions[name]='TagBelow:'+support_tags[name][field]
    return definitions


def fluid_tag_resources(jar):
    with zipfile.ZipFile(jar) as archive:
        prefix='data/minecraft/tags/fluid/'
        return {'minecraft:'+name[len(prefix):-5]:archive.read(name).decode('utf-8') for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith('.json')}


def behavior_key(rule,value):
    properties=value.get('properties',{})
    return (rule,properties.get('facing') if rule in ['FullBehind','SolidBehind','Cocoa','PistonHead','Bell'] or rule=='FaceAttached' and properties.get('face')=='wall' else None,properties.get('hanging') if rule in ['Lantern','MangrovePropagule'] else None,properties.get('bottom') if rule=='MossCarpet' else None,properties.get('face') if rule=='FaceAttached' else None,properties.get('half') if rule in ['Door','DoublePlant','SmallDripleaf','TallSeagrass','PitcherCrop'] else None,properties.get('vertical_direction') if rule=='Speleothem' else None,properties.get('type') if rule=='PistonHead' else None,value['id'] if rule in ['GrowingPlant','Speleothem'] else None,properties.get('attachment') if rule=='Bell' else None,tuple(properties.get(face) for face in ['down','up','north','south','west','east']) if rule in ['Multiface','Vine'] else None)

def corpus(facts,definitions):
    positions=[[0,0,0],[2147483647,-2147483648,2147483647],[-17,2147483647,-33]]
    def case(state,neighbors,position=0,**kwargs):
        f={'state':state,'neighbors':neighbors,'position':positions[position%3],'entity':'none','fail':-1,'entity_fail':-1};f.update(kwargs);return f
    defaults=facts['defaults'];samples=[defaults[name] for name in ['minecraft:air','minecraft:stone','minecraft:water','minecraft:pale_moss_carpet'] if name in defaults]
    representatives={}
    for state,value in enumerate(facts['states']):
        rule=definitions.get(value['id'])
        if rule is None:continue
        yield case(state,samples or [0],state,fail=0)
        if rule=='Always':continue
        for neighbor in [*samples,state,defaults[value['id']]]:yield case(state,[neighbor],state)
        key=behavior_key(rule,value)
        representatives.setdefault(key,state)
    special=[defaults[name] for name in ['minecraft:shulker_box','minecraft:moving_piston','minecraft:scaffolding','minecraft:oak_slab','minecraft:stone','minecraft:air','minecraft:pale_moss_carpet','minecraft:bamboo'] if name in defaults] or [0]
    for candidate in representatives.values():
        rule=definitions[facts['states'][candidate]['id']]
        for neighbor in range(len(facts['states'])):yield case(candidate,[neighbor],neighbor)
        extra=['big_dripleaf','big_dripleaf_stem','clay','dirt'] if rule=='BigDripleafStem' else ['oak_fence_gate','dirt','oak_trapdoor'] if rule=='Farmland' else ['farmland','dirt'] if rule in ['Crop','PitcherCrop'] else []
        neighbors=[*special,*[defaults['minecraft:'+name] for name in extra if 'minecraft:'+name in defaults]]
        if rule in ['Crop','PitcherCrop']:
            for brightness in [-2147483648,-1,0,7,8,9,15,2147483647]:
                for neighbor in neighbors:
                    for fail in [-1,0]:
                        for light_fail in [-1,0]:yield case(candidate,[neighbor],candidate,brightness=brightness,fail=fail,light_fail=light_fail)
        if rule in ['LilyPad','Frogspawn','SporeBlossom','SmallDripleaf','TallSeagrass','SugarCane']:
            fluid_states=[index for index,state in enumerate(facts['states']) if state['id'] in ['minecraft:water','minecraft:lava','minecraft:air']]
            supporting=[defaults['minecraft:'+name] for name in ['stone','dirt','clay','ice','big_dripleaf','small_dripleaf','tall_seagrass','sugar_cane','oak_leaves'] if 'minecraft:'+name in defaults]
            for first in fluid_states:
                for second in [defaults[name] for name in ['minecraft:air','minecraft:water','minecraft:lava'] if name in defaults]:
                    for neighbor in supporting:
                        for fail in [-1,0,1,4]:
                            for fluid_fail in [-1,0,1,3]:
                                yield case(candidate,[neighbor],candidate,fluid_states=[first,second],fail=fail,fluid_fail=fluid_fail)
        if rule=='Fire':
            air=defaults['minecraft:air']
            # An empty first support query forces the ignition lookup even when
            # the subsequent neighbor has a full supporting face.
            for neighbor in range(len(facts['states'])):
                yield case(candidate,[air,neighbor],neighbor)
        if rule=='Mushroom':
            for brightness in [-2147483648,0,12,13,14,2147483647]:
                for neighbor in range(len(facts['states'])):
                    yield case(candidate,[neighbor],neighbor,brightness=brightness)
            for neighbor in range(0,len(facts['states']),7):
                for fail in [-1,0]:
                    yield case(candidate,[neighbor],neighbor,brightness=12,fail=fail,light_fail=0)
        if rule=='Cactus':
            air=defaults['minecraft:air'];cactus=defaults['minecraft:cactus']
            for neighbor in range(len(facts['states'])):
                yield case(candidate,[air,air,air,air,cactus,neighbor],neighbor,fluid_states=[air])
            supports=[defaults['minecraft:'+name] for name in ['air','sand','red_sand','cactus','stone'] if 'minecraft:'+name in defaults]
            fluid_states=[index for index,state in enumerate(facts['states']) if state['id'] in ['minecraft:air','minecraft:water','minecraft:lava']]
            for support in supports:
                for fluid in fluid_states:
                    for slot in range(4):
                        fluids=[air]*4;fluids[slot]=fluid
                        for fail in [-1,*range(6)]:
                            for fluid_fail in [-1,*range(4)]:
                                yield case(candidate,[air,air,air,air,support,air],slot,fluid_states=fluids,fail=fail,fluid_fail=fluid_fail)
        if rule in ['Multiface','Vine','HangingMoss','SnowLayer','SeaPickle']:
            shulkers=[index for index,state in enumerate(facts['states']) if state['id']=='minecraft:shulker_box']
            for neighbor in shulkers:
                for old,current in [(0,0),(0,1),(1,0),(0.1,0.2),(0.7,0.8),(1,1)]:
                    for entity in ['closed','opened']:
                        for entity_fail in [-1,0,1,2,3]:
                            yield case(candidate,[neighbor],candidate,entity=entity,entity_fail=entity_fail,shulker_progress=[struct.pack('>f',v).hex() for v in [old,current]])
        if rule in ['ChorusPlant','ChorusFlower','Scaffolding','Multiface','Vine','HangingMoss','SnowLayer','SeaPickle','Fire','Cactus']:
            # Long read sequences expose directional order, early exits and failures
            # beyond the two alternating neighbors used by the general corpus.
            names=['air','stone','end_stone','chorus_plant','chorus_flower','vine','pale_hanging_moss','shulker_box','scaffolding','oak_slab','oak_planks','sand','cactus','water']
            palette=[defaults['minecraft:'+name] for name in names if 'minecraft:'+name in defaults]
            if rule=='Scaffolding':
                palette.extend(index for index,state in enumerate(facts['states']) if state['id']=='minecraft:scaffolding')
            rng=random.Random(0x484f52495a4f4e+candidate)
            sequences=(tuple(rng.choice(palette) for _ in range(10)) for _ in range(4096 if rule in ["ChorusPlant","ChorusFlower","Scaffolding"] else 512))
            for index,sequence in enumerate(sequences):
                for fail in [-1,*range(10)]:yield case(candidate,list(sequence),index,fail=fail)
            if rule in ['ChorusPlant','ChorusFlower']:
                for index,sequence in enumerate(itertools.product(palette[:5],repeat=5)):
                    yield case(candidate,list(sequence),index)
        for first in neighbors:
            for second in neighbors:
                for fail in [-1,0,1]:
                    for entity_fail in [-1,0]:
                        for entity in ['none','closed','opened']:yield case(candidate,[first,second],candidate,fail=fail,entity_fail=entity_fail,entity=entity)


def save_checkpoint(path, checkpoint):
    temporary=path.with_suffix('.tmp')
    temporary.write_text(json.dumps(checkpoint,indent=2)+'\n',encoding='utf-8')
    temporary.replace(path)


def load_checkpoint(path, sources, environment):
    if not path.exists():return None
    checkpoint=json.loads(path.read_text(encoding='utf-8'))
    if checkpoint.get('schema')!=1 or checkpoint['sources_sha256']!=sources or checkpoint['environment']!=environment:
        raise ValueError('checkpoint source/environment changed; run again without --resume')
    for group in ['fixtures_sha256','artifacts_sha256']:
        for name,digest in checkpoint[group].items():
            artifact=Path(name)
            if not artifact.is_file() or lab.digest(artifact,'sha256')!=digest:
                raise ValueError(f'checkpoint artifact changed: {artifact}; run again without --resume')
    return checkpoint


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare',action='store_true')
    parser.add_argument('--resume',action='store_true',help='Reuse completed stages only when all recorded input and executable hashes still match')
    args=parser.parse_args()
    build=lab.LOCAL/'rust-block-survival-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    checkpoint_path=build/'run-state.json'
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockSurvivalOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaBlockPropertiesOracle','VanillaBlockSupportOracle','VanillaBlockStateCodecOracle','VanillaCollisionOracle','VanillaHeightmapOracle']]
    sources=[oracle,*helpers,lab.ROOT/'tools/test_worldgen_parity.py',Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/block_survival_fixture.rs']
    tested_sources=base.source_hashes(sources)
    environment={'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'dependencies':{str(path):lab.digest(path,'sha256') for path in [*jars.values(),*libraries]}}
    checkpoint=load_checkpoint(checkpoint_path,tested_sources,environment) if args.resume else None
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,stdout=log,stderr=subprocess.STDOUT,timeout=3600)
    dependency_path=os.pathsep.join(map(str,libraries))
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBlockSurvivalOracle',mode,path],side+'-'+mode)
    if checkpoint is None:
        checkpoint_path.unlink(missing_ok=True)
        tags=worldgen.tag_resources(jars['server'])
        if tags!=worldgen.tag_resources(jars['client']):raise ValueError('client/server tags differ')
        (build/'tags.json').write_text(json.dumps(tags,separators=(',',':')))
        fluid_tags=fluid_tag_resources(jars['server'])
        if fluid_tags!=fluid_tag_resources(jars['client']):raise ValueError('client/server fluid tags differ')
        (build/'fluid-tags.json').write_text(json.dumps(fluid_tags,separators=(',',':')))
        run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
        java('server','export',build)
        facts=json.loads((build/'facts.json').read_text());definition_classes=json.loads((build/'survival-classes.json').read_text())
        definitions=bind_definitions(definition_classes,json.loads((build/'survival-placement-classes.json').read_text()),json.loads((build/'survival-support-tags.json').read_text()))
        if len(facts['states'])!=35723:raise ValueError('incomplete pinned registry')
        if len(definitions)!=len(facts['defaults']):raise ValueError('incomplete pinned survival definitions')
        (build/'survival-rules.json').write_text(json.dumps(definitions,separators=(',',':')))
        inputs=build/'inputs.txt';fixture_count=0
        with inputs.open('w',encoding='utf-8') as output:
            for fixture in corpus(facts,definitions):
                output.write(json.dumps(fixture,separators=(',',':'))+'\n');fixture_count+=1
        fixture_paths=[inputs,build/'survival-extra-definitions.json',build/'fluid-tags.json',build/'survival-placement-classes.json',build/'survival-support-tags.json',build/'facts.json',build/'survival-classes.json',build/'survival-rules.json',build/'tags.json',build/'block-registry.nbt',build/'cached-collision.nbt',build/'block-properties.nbt',build/'cached-support.nbt']
        tested_fixtures=base.source_hashes(fixture_paths)
        run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
        metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
        binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
        base.ensure_sources_unchanged(sources,tested_sources)
        checkpoint={'schema':1,'environment':environment,'sources_sha256':tested_sources,'fixtures_sha256':{str(lab.ROOT/name):digest for name,digest in tested_fixtures.items()},'artifacts_sha256':{str(path):lab.digest(path,'sha256') for path in [binary,*sorted(classes.rglob('*.class'))]},'binary':str(binary),'counts':{'supported_blocks':len(definitions),'supported_states':sum(state['id'] in definitions for state in facts['states']),'survival_rules':len(set(definitions.values())),'survival_rule_kinds':len({rule.split(':',1)[0] for rule in definitions.values()}),'fluid_tag_files':len(fluid_tags),'fixtures':fixture_count},'outputs':{}}
        save_checkpoint(checkpoint_path,checkpoint)
        print('Verified inputs and compiled executables checkpointed',flush=True)
    def stage(name, action):
        output=build/(name+'.txt')
        previous=checkpoint['outputs'].get(name)
        if previous is None or not output.exists() or lab.digest(output,'sha256')!=previous:
            action()
            checkpoint['outputs'][name]=lab.digest(output,'sha256')
            save_checkpoint(checkpoint_path,checkpoint)
            print(f'{name}: completed output checkpointed',flush=True)
    stage('rust',lambda:run([checkpoint['binary'],'block_survival_fixture',build],'rust'))
    references=[]
    for side,jar in jars.items():
        stage(side,lambda:java(side,side,build))
        reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} block-survival output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources)
    load_checkpoint(checkpoint_path,tested_sources,environment)
    report={'schema':1,**{key:value for key,value in environment.items() if key!='dependencies'},**checkpoint['counts'],'sources_sha256':tested_sources,'fixtures_sha256':{str(Path(name).relative_to(lab.ROOT)):digest for name,digest in checkpoint['fixtures_sha256'].items()},'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Version-pinned block survival rules are assigned from privately inspected native definition classes and executed in Rust against solid/air/tag tests and unified support geometry. Actual native canSurvive evaluates every supported candidate state, each neighbor state against behavior representatives, changing repeated reads, world-coordinate wrapping, absent/closed/open shulker snapshots, brightness thresholds, independent fluid state sequences and block/entity/light/fluid query failures. All pinned survival families are included; full block placement/update behavior, world/entity lifecycle and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped block-survival parity passed: {report_path}')


if __name__=='__main__':main()
