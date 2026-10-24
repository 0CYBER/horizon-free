#!/usr/bin/env python3
"""Compare simple-block feature effects and separate random streams with vanilla."""
import argparse
import itertools
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen, test_block_survival_parity as survival
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_block_survival_parity as survival


def corpus(facts, placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    encode=lambda obj:json.dumps(obj,separators=(',',':'))
    def case(candidate=None,**extra):
        origin=extra.pop('origin',[0,64,0])
        def at(dy):return [origin[0],((origin[1]+dy+2**31)%2**32)-2**31,origin[2]]
        f={'definition':encode({'to_place':states[candidate if candidate is not None else get('stone')],'schedule_tick':True}),'origin':origin,'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,'brightness':15,'fallback':get('stone'),'states':[[*at(0),get('air')],[*at(1),get('air')],[*at(-1),get('dirt')]],'repeats':2}
        f.update(extra);return f
    origins=[[0,64,0],[2147483647,-2147483648,2147483647],[-17,2147483647,-33]]
    for candidate in range(len(states)):yield case(candidate,origin=origins[candidate%3])
    double_names=set(placement['double_plants']);representatives={}
    for candidate,state in enumerate(states):
        if state['id'] not in double_names:continue
        key=(state['id'],state.get('properties',{}).get('waterlogged'))
        if state.get('properties',{}).get('half')=='lower':representatives.setdefault(key,candidate)
        below=next(i for i,s in enumerate(states) if s['id']==state['id'] and s.get('properties',{}).get('half')=='lower') if state.get('properties',{}).get('half')=='upper' else get('clay') if state['id']=='minecraft:small_dripleaf' else get('dirt')
        for above in [get(name) for name in ['air','cave_air','void_air','stone','water','lava','short_grass','snow','small_dripleaf','tall_seagrass','pale_moss_carpet']]:
            for fluid in [get('air'),get('water'),get('lava')]:
                for fail in [-1,*range(9)]:yield case(candidate,states=[[0,63,0,below],[0,64,0,get('air')],[0,65,0,above]],fluid=fluid,fail=fail,reject=3)
    for candidate in representatives.values():
        name=states[candidate]['id'];below=get('clay') if name=='minecraft:small_dripleaf' else get('farmland') if name=='minecraft:pitcher_crop' else get('dirt')
        for above in range(len(states)):yield case(candidate,states=[[0,63,0,below],[0,64,0,get('air')],[0,65,0,above]],fluid=get('water'),repeats=1)
    moss_ids=[i for i,s in enumerate(states) if s['id']=='minecraft:pale_moss_carpet'];sides=[(0,-1),(1,0),(0,1),(-1,0)]
    for lower_mask,upper_mask in itertools.product(range(16),repeat=2):
        for legacy in [False,True]:
            for seed in [0,-1,0x123456789abcdef]:
                for fail in [-1,0,3,7,12,19,25]:
                    entries=[[0,63,0,get('stone')],[0,64,0,get('air')],[0,65,0,get('air')]]
                    for y,mask in [(64,lower_mask),(65,upper_mask)]:entries.extend([x,y,z,get('stone') if mask&(1<<index) else get('air')] for index,(x,z) in enumerate(sides))
                    yield case(get('pale_moss_carpet'),states=entries,legacy=legacy,seed=seed,world_seed=seed,fail=fail,fallback=get('air'),reject=0 if seed==0 else 3)
    for lower,upper in itertools.product(moss_ids,moss_ids):yield case(lower,states=[[0,63,0,get('stone')],[0,64,0,get('air')],[0,65,0,upper],[0,66,0,lower]],world_seed=upper,reject=0 if lower%2 else 3)
    rng=random.Random(0x53494d504c45);palette=[get(name) for name in ['shulker_box','scaffolding','snow','piston_head','moving_piston','powder_snow','bamboo','stone','air','oak_slab','oak_stairs']]
    for _ in range(4096):
        entries=[[0,63,0,get('stone')],[0,64,0,get('air')],[0,65,0,rng.choice(moss_ids)]]
        entries.extend([x,y,z,rng.choice(palette)] for y in [64,65] for x,z in sides)
        yield case(get('pale_moss_carpet'),states=entries,seed=rng.randrange(2**63),world_seed=rng.randrange(2**63),fail=rng.randrange(-1,25),fallback=get('air'))
    providers=[{'type':'random_block','blocks':[]},{'type':'random_block','blocks':['stone','small_dripleaf','pale_moss_carpet']},{'type':'weighted','entries':[{'data':'stone','weight':1},{'data':'pale_moss_carpet','weight':3}]},{'type':'rotated','state':{'id':'oak_log'}},{'type':'rule_based','rules':[]},{'type':'rule_based','rules':[{'if_true':{'type':'true'},'then':{'id':'pale_moss_carpet'}}]},'horizon:stone']
    for provider in providers:
        for legacy in [False,True]:
            for seed in [0,1,-1,0x123456789abcdef]:
                for fail in [-1,*range(12)]:
                    for reject in [0,1,3]:yield case(definition=encode({'to_place':provider,'schedule_tick':True}),legacy=legacy,seed=seed,world_seed=seed,fail=fail,reject=reject,repeats=4)
    for schedule in [None,False,True,0,1,'true',{},[]]:yield case(definition=encode({'to_place':{'id':'stone'},'schedule_tick':schedule}))
    for definition in [None,False,0,[],{}, {'to_place':None},{'to_place':'missing'},{'to_place':{'id':'stone'}},{'to_place':{'id':'missing'}},{'to_place':{'type':'random_block','blocks':[]},'ignored':1}]:yield case(definition=encode(definition))

    simple=lambda provider:{'type':'simple_block','to_place':provider,'schedule_tick':True}
    placed=lambda feature,modifiers=[]:{'feature':feature,'placement':modifiers}
    moss=placed('horizon:moss',[{'type':'count','count':2}]);empty=placed('horizon:empty');stone=placed('horizon:stone')
    graphs=['horizon:moss',simple('horizon:stone'),
        {'type':'sequence','features':[moss,stone]},
        {'type':'sequence','features':[moss,empty,stone]},
        {'type':'simple_random_selector','features':[moss,empty,stone]},
        {'type':'random_boolean_selector','feature_true':moss,'feature_false':empty},
        {'type':'weighted_random_selector','features':[{'data':moss,'weight':1},{'data':empty,'weight':3},{'data':stone,'weight':2}]},
        {'type':'random_selector','features':[{'feature':moss,'chance':0.4},{'feature':empty,'chance':0.7}],'default':stone}]
    chains=[[],[{'type':'count','count':2}],[{'type':'count','count':2},{'type':'in_square'}],
        [{'type':'random_chance','chance':0.5},{'type':'count','count':2}],
        [{'type':'offset','x':1,'y':1,'z':-1}],
        [{'type':'block_predicate_filter','predicate':{'type':'would_survive','state':'pale_moss_carpet'}}]]
    for graph in graphs:
        for mode,definition in [('feature',graph),*[('placed',placed(graph,chain)) for chain in chains]]:
            for legacy in [False,True]:
                for seed in [0,1,-1,0x123456789abcdef]:
                    for fail in [-1,0,3,7,19]:
                        for reject in [0,1,3]:yield case(definition=encode(definition),mode=mode,legacy=legacy,seed=seed,world_seed=seed,fail=fail,reject=reject,repeats=3)
    for name in ['horizon:moss','horizon:stone','horizon:empty','horizon:missing']:
        yield case(definition=encode(name),mode='placed')


def run(corpus, build_name, source_file, scope):
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    label=build_name.removeprefix('rust-').removesuffix('-parity')
    build=lab.LOCAL/build_name;build.mkdir(parents=True,exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts();oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSimpleBlockOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaPlacementOracle','VanillaBlockSurvivalOracle','VanillaBlockPropertiesOracle','VanillaBlockSupportOracle','VanillaBlockStateCodecOracle','VanillaCollisionOracle','VanillaHeightmapOracle']]
    sources=[oracle,*helpers,Path(__file__),Path(source_file),*[lab.ROOT/'tools'/name for name in ['test_block_survival_parity.py','test_worldgen_parity.py','test_rust_parity.py','lab.py']],lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/simple_block_fixture.rs']
    tested_sources=base.source_hashes(sources);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    def java(side,mode,path):run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaSimpleBlockOracle',mode,path],side+'-'+mode)
    tags=worldgen.tag_resources(jars['server']);fluid_tags=survival.fluid_tag_resources(jars['server'])
    if tags!=worldgen.tag_resources(jars['client']) or fluid_tags!=survival.fluid_tag_resources(jars['client']):raise ValueError('reference tags differ')
    for name,value in [('tags.json',tags),('fluid-tags.json',fluid_tags),('catalog.json',{'horizon:stone':json.dumps({'id':'stone'})})]:(build/name).write_text(json.dumps(value,separators=(',',':')))
    feature_bundle={'features':{name:json.dumps({'type':'simple_block','to_place':provider,'schedule_tick':True}) for name,provider in [('horizon:moss',{'id':'pale_moss_carpet'}),('horizon:stone','horizon:stone'),('horizon:empty',{'type':'random_block','blocks':[]})]},'placed':{name:json.dumps({'feature':name,'placement':[{'type':'count','count':2}]}) for name in ['horizon:moss','horizon:stone','horizon:empty']}}
    (build/'feature-catalog.json').write_text(json.dumps(feature_bundle,separators=(',',':')))
    run(['javac','--release' ,lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
    for side in jars:
        export=build/(side+'-definitions');export.mkdir(exist_ok=True);java(side,'export',export)
    for path in (build/'server-definitions').iterdir():
        if path.read_bytes()!=(build/'client-definitions'/path.name).read_bytes():raise ValueError('reference definitions differ: '+path.name)
        (build/path.name).write_bytes(path.read_bytes())
    facts=json.loads((build/'facts.json').read_text());placement=json.loads((build/'placement-definitions.json').read_text())
    rules=survival.bind_definitions(*[json.loads((build/name).read_text()) for name in ['survival-classes.json','survival-placement-classes.json','survival-support-tags.json']]);(build/'survival-rules.json').write_text(json.dumps(rules,separators=(',',':')))
    inputs=build/'inputs.txt';fixture_count=0
    with inputs.open('w',encoding='utf-8') as output:
        for fixture in corpus(facts,placement):output.write(json.dumps(fixture,separators=(',',':'))+'\n');fixture_count+=1
    fixtures=[inputs,*sorted(build.glob('*.nbt')),*[p for p in sorted(build.glob('*.json')) if p.name!='report.json']];tested_fixtures=base.source_hashes(fixtures);print(f'Prepared {fixture_count:,} placement fixtures',flush=True)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'simple_block_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);matched=base.compare(build/(side+'.txt'),build/'rust.txt');print(f'{side}: {matched:,} {label} output lines match Rust',flush=True)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched})
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixtures,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':fixture_count,'candidate_states':len(facts['states']),'double_plant_blocks':len(placement['double_plants']),'mossy_carpet_blocks':len(placement['mossy_carpets']),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':scope}
    lab.write_json(report_path,report);print(f'Scoped {label} parity passed: {report_path}',flush=True)


def main():
    run(corpus,'rust-simple-block-parity',__file__,'Native SimpleBlockFeature map codec, provider selection, actual state survival and single/double-plant/moss placement. Named feature/placed-feature/provider holders, selectors, sequences and placement modifier chains compose the same operations. Mutable world logs ordered reads, fluid/light/entity queries, writes and flags, rejected writes, thrown failures, scheduled block identities, repeated placements and both feature/world random tails. All candidate states and every above-state obstruction for representative double-plant definitions, waterlogging, moss attachment combinations and topper identities are exercised. Registry definition classes and immutable geometry are independently checked in both authenticated JARs. Actual chunks, neighbor updates, scheduled tick execution, entity lifecycle, complete feature registries and playable worlds remain pending.')


if __name__=='__main__':main()
