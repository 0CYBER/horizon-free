#!/usr/bin/env python3
"""Compare vanilla tree, fallen-tree and ore features block-for-block with vanilla."""
import argparse
import json
import re
import os
from pathlib import Path
import platform
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen, test_block_survival_parity as survival
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_block_survival_parity as survival


def feature_resources(jar):
    """Vanilla configured features and named state providers from the authenticated JAR."""
    result={'features':{},'providers':{},'placed':{}}
    with zipfile.ZipFile(jar) as archive:
        for kind,key in [('feature','features'),('block_state_provider','providers'),('placed_feature','placed')]:
            prefix=f'data/minecraft/worldgen/{kind}/'
            for name in sorted(archive.namelist()):
                if name.startswith(prefix) and name.endswith('.json'):
                    result[key]['minecraft:'+name[len(prefix):-5]]=archive.read(name).decode('utf-8')
    return result


def fixture_hashes(paths):
    # Build outputs may live behind a linked private directory outside ROOT.
    return {path.name:lab.digest(path,'sha256') for path in paths}


def corpus(facts,resources):
    get=lambda name:facts['defaults']['minecraft:'+name]
    def fixture(feature,**extra):
        f={'feature':feature,'seed':0,'legacy':False,'origin':[8,65,8],'height':65,'layers':[[59,get('stone')],[63,get('dirt')],[64,get('grass_block')]],
           'hole_mod':0,'hole_state':get('air'),'obstacle_mod':0,'obstacle_state':get('air'),'obstacle_min':0,'states':[]}
        f.update(extra);f.setdefault('world_seed',f['seed']^0x2545F4914F6CDD1D);return f
    seeds=[0,1,-1,0x5DEECE66D,0x123456789abcdef,-8070450532247928832]
    origins=[[8,65,8],[15,65,0],[0,65,15],[-1,65,-17],[31,65,-16],[100003,65,-4097]]
    trees,ores,others=[],[],[]
    def needs_level(text,seen=frozenset()):
        # Pale moss needs the level's feature registry and server level; templates need structure
        # resources. Both are excluded directly or through named features.
        if 'pale_moss' in text or '"minecraft:template"' in text:return True
        for ref in set(re.findall(r'"(minecraft:[a-z0-9_/]+)"',text))-seen:
            for table in ('features','placed'):
                if ref in resources.get(table,{}) and needs_level(resources[table][ref],seen|{ref}):return True
        return False
    for name,text in resources['features'].items():
        value=json.loads(text);kind=value['type'].removeprefix('minecraft:')
        if needs_level(text):continue
        if kind in ('tree','fallen_tree'):
            trees.append(name)
        elif kind in ('ore','scattered_ore'):ores.append(name)
        elif kind not in ('freeze_top_layer','template','fossil'):others.append(name)  # templates need structure resources
    worlds=[
        {},
        {'obstacle_mod':37,'obstacle_state':get('oak_leaves'),'obstacle_min':66},
        {'obstacle_mod':23,'obstacle_state':get('stone'),'obstacle_min':65},
        {'obstacle_mod':19,'obstacle_state':get('vine'),'obstacle_min':65},
        {'layers':[[59,get('stone')],[64,get('mud')],[67,get('water')]],'height':68},
        {'layers':[[300,get('stone')],[310,get('dirt')],[311,get('grass_block')]],'origin_y':312,'height':312},
        {'layers':[[59,get('stone')],[63,get('dirt')],[64,get('podzol')]],'hole_mod':11,'hole_state':get('short_grass')},
    ]
    for name in trees:
        for w,world in enumerate(worlds):
            for s,seed in enumerate(seeds):
                extra=dict(world);origin=list(origins[(s+w)%len(origins)]);origin[1]=extra.pop('origin_y',extra.get('height',65))
                yield fixture(name,seed=seed,legacy=s%3==2,origin=origin,**extra)
    ore_worlds=[
        {'layers':[[-1,get('deepslate')],[250,get('stone')]],'height':251},
        {'layers':[[-1,get('deepslate')],[250,get('stone')]],'height':251,'hole_mod':5,'hole_state':get('air')},
        {'layers':[[-1,get('deepslate')],[250,get('stone')]],'height':-100},
        {'layers':[[250,get('netherrack')]],'height':251,'hole_mod':9,'hole_state':get('cave_air')},
        {'layers':[[30,get('tuff')],[250,get('granite')]],'height':251,'hole_mod':13,'hole_state':get('water')},
    ]
    for name in ores:
        for w,world in enumerate(ore_worlds):
            for s,seed in enumerate(seeds):
                origin=list(origins[(s+w)%len(origins)]);origin[1]=[-60,-3,0,40,316,-64][(s*5+w)%6]
                yield fixture(name,seed=seed,legacy=s%3==2,origin=origin,**world)

    air,water,stone=get('air'),get('water'),get('stone')
    generic_worlds=[
        ({},[65]),
        ({'layers':[[200,stone]],'height':201,'boxes':[[-40,20,-40,56,35,56,air]]},[20,35,27]),
        ({'layers':[[200,stone]],'height':201,'boxes':[[-40,23,-40,56,35,56,air],[-40,20,-40,56,22,56,water]]},[21,23]),
        ({'layers':[[35,stone],[39,get('sand')],[62,water]],'height':40},[40,63]),
        ({'layers':[[31,get('lava')],[100,get('netherrack')]],'height':101,'boxes':[[-40,32,-40,56,60,56,air]]},[32,60]),
    ]
    for name in others:
        for w,(world,ys) in enumerate(generic_worlds):
            for y in ys:
                for s,seed in enumerate(seeds[:4]):
                    origin=list(origins[(s+w)%len(origins)]);origin[1]=y
                    yield fixture(name,seed=seed,legacy=s%3==2,origin=origin,**world)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-tree-feature-parity';build.mkdir(parents=True,exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts();parity=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity'
    oracle=parity/'VanillaTreeFeatureOracle.java';exporter=parity/'VanillaSimpleBlockOracle.java'
    helpers=[parity/(name+'.java') for name in ['VanillaPlacementOracle','VanillaBlockSurvivalOracle','VanillaBlockPropertiesOracle','VanillaBlockSupportOracle','VanillaBlockStateCodecOracle','VanillaCollisionOracle','VanillaHeightmapOracle']]
    sources=[oracle,exporter,*helpers,Path(__file__),*[lab.ROOT/'tools'/name for name in ['test_block_survival_parity.py','test_worldgen_parity.py','test_rust_parity.py','lab.py']],lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',
             *sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/tree_feature_fixture.rs']
    tested_sources=base.source_hashes(sources);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=7200,stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    def java(main_class,side,mode,path):run(['java','-Xmx2G','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.'+main_class,mode,path],side+'-'+mode)
    tags=worldgen.tag_resources(jars['server']);fluid_tags=survival.fluid_tag_resources(jars['server']);resources=feature_resources(jars['server'])
    if tags!=worldgen.tag_resources(jars['client']) or fluid_tags!=survival.fluid_tag_resources(jars['client']) or resources!=feature_resources(jars['client']):raise ValueError('reference resources differ')
    for name,value in [('tags.json',tags),('fluid-tags.json',fluid_tags),('vanilla-features.json',resources)]:(build/name).write_text(json.dumps(value,separators=(',',':')))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,exporter,*helpers],'compile-java')
    for side in jars:
        export=build/(side+'-definitions');export.mkdir(exist_ok=True);java('VanillaSimpleBlockOracle',side,'export',export)
    for path in (build/'server-definitions').iterdir():
        if path.name=='inputs.txt':continue  # survival export's own corpus
        if path.read_bytes()!=(build/'client-definitions'/path.name).read_bytes():raise ValueError('reference definitions differ: '+path.name)
        (build/path.name).write_bytes(path.read_bytes())
    facts=json.loads((build/'facts.json').read_text())
    rules=survival.bind_definitions(*[json.loads((build/name).read_text()) for name in ['survival-classes.json','survival-placement-classes.json','survival-support-tags.json']]);(build/'survival-rules.json').write_text(json.dumps(rules,separators=(',',':')))
    inputs=build/'inputs.txt';count=0
    with inputs.open('w',encoding='utf-8') as output:
        only=os.environ.get('FEATURE_FILTER')  # optional regex over feature names for focused reruns
        for fixture in corpus(facts,resources):
            if only and not __import__('re').search(only,fixture['feature']):continue
            output.write(json.dumps(fixture,separators=(',',':'))+'\n');count+=1
    fixtures=[inputs,*sorted(build.glob('*.nbt')),*[p for p in sorted(build.glob('*.json')) if p.name!='report.json']];tested_fixtures=fixture_hashes(fixtures)
    print(f'Prepared {count:,} feature fixtures',flush=True)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'tree_feature_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java('VanillaTreeFeatureOracle',side,side,build);matched=base.compare(build/(side+'.txt'),build/'rust.txt');print(f'{side}: {matched:,} tree/ore output lines match Rust',flush=True)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched})
    base.ensure_sources_unchanged(sources,tested_sources);
    if fixture_hashes(fixtures)!=tested_fixtures:raise ValueError('fixtures changed during the run')
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],
            'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':count,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,
            'scope':'Every vanilla 26.3 minecraft:tree (except pale-moss decorated), minecraft:fallen_tree, minecraft:ore and minecraft:scattered_ore configured feature, decoded from the authenticated JAR data, placed with both random kinds over layered synthetic worlds (open ground, leaf/stone/vine obstacles, flooded mud, near the build limit, podzol with grass) backed by native proto-chunk sections. Ordered level writes and flags, beehive occupants, post-processing marks, results, random tails and the final block diff of every touched chunk are compared, including native leaf distances and edge shape updates. Pale-moss ground patches, real WorldGenRegion write policy, heightmaps and decoration scheduling remain covered elsewhere or pending.'}
    lab.write_json(report_path,report);print(f'Scoped tree/ore feature parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
