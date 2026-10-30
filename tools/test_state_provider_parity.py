#!/usr/bin/env python3
"""Compare initial state-provider codecs, choices and optional sampling."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import zipfile
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen, test_biome_temperature_parity as temperature, test_material_condition_parity as material, test_block_survival_parity as survival
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_biome_temperature_parity as temperature
    import test_material_condition_parity as material, test_block_survival_parity as survival


def corpus(facts,tags,provider_resources=None,catalog_resources=None,fluid_tags=None,biomes=None,biome_tags=None,support_states=None,survival_rules=None):
    def cases(definition,states=None,repeats=1,wide=False):
        if states is None:states=list(facts['defaults'].values())[:6]
        for typed in [False,True]:
            for legacy,worldgen in ([(False,False),(True,False),(False,True),(True,True)] if wide else [(False,False)]):
                yield {'definition':json.dumps(definition,separators=(',',':')),'typed':typed,'legacy':legacy,'worldgen':worldgen,'seed':-1 if legacy else 0x123456789abcdef,'states':states,'repeats':repeats,'fail':-1}
    for index,state in enumerate(facts['states']):
        yield from cases(state,[index])
        yield from cases({'type':'simple','state':state},[index])
    for name in facts['defaults']:
        for value in [name,{'type':'simple','state':name},{'type':'random_block','blocks':name},{'type':'random_block','blocks':[name]}]:
            yield from cases(value,[0],wide=False)
    for name in sorted(tags):
        yield from cases({'type':'random_block','blocks':'#'+name},wide=True)
    for blocks in [[],['water','stone','water'],['air'],['oak_log','redstone_wire','water'],['#minecraft:logs'],'#horizon:missing',['missing'],False,{},None]:
        for f in cases({'type':'random_block','blocks':blocks},repeats=3,wide=True):
            yield f
            yield dict(f,fail=0)
            yield dict(f,fail=2)
    entries=lambda weights:[{'data':state,'weight':weight} for state,weight in zip(['stone','water','oak_log'],weights)]
    for weights in [[],[0],[1],[100],[0,0,0],[0,1,0],[1,0,2],[2147483647],[2147483647,1],[-1],[1.9],[4294967297],[True],[None]]:
        yield from cases({'type':'weighted','entries':entries(weights)},repeats=5,wide=True)
    for state in ['missing','stone',{'id':'water','properties':{'level':'+01'}},{'id':'oak_log','properties':{'axis':'z'}},None,False,1,[],{}]:
        yield from cases({'type':'simple','state':state},wide=True)
    for typed in [{'type':'simple','state':'water'},{'type':'weighted','entries':entries([1,2,3])},{'type':'random_block','blocks':['water']},{'type':'missing'}]:
        for name in ['stone','missing']:
            yield from cases(dict(typed,id=name),wide=True)
    all_states=list(range(len(facts['states'])))
    for direction in ['down','up','north','south','west','east']:
        yield from cases({'type':'rotated','state':{'type':'random_block','blocks':[]},'direction':direction},all_states)
    yield from cases({'type':'rotated','state':{'type':'random_block','blocks':[]}},all_states,wide=True)
    sources=[{'id':'oak_log'},{'type':'weighted','entries':[{'data':'oak_log','weight':2},{'data':'piston','weight':3},{'data':'hopper','weight':1}]},{'type':'random_block','blocks':['furnace','stone','oak_stairs']},{'type':'random_block','blocks':[]},{'type':'random_block','blocks':'#horizon:empty'},'oak_log',None]
    for source in sources:
        for direction in [None,'up','west','UP','missing',False,1]:
            for f in cases({'type':'rotated','state':source,'direction':direction},repeats=4,wide=True):
                yield f
                yield dict(f,fail=0)
    for depth in [1,2,8,64]:
        source={'type':'weighted','entries':[{'data':'oak_log','weight':1},{'data':'piston','weight':1}]}
        for _ in range(depth):source={'type':'rotated','state':source}
        yield from cases(source,repeats=2,wide=True)
    # One representative per property-family value covers every compatible
    # property update for each target state without a quadratic all-state matrix.
    integers={tuple(v) for v in facts.get('integers',[])}
    domains={}
    for state in facts['states']:
        for name,value in state.get('properties',{}).items():domains.setdefault((state['id'],name),set()).add(value)
    signatures={key:(key[1],key in integers,tuple(sorted(values))) for key,values in domains.items()}
    representatives={}
    for index,state in enumerate(facts['states']):
        for name,value in state.get('properties',{}).items():representatives.setdefault(signatures[(state['id'],name)],{}).setdefault(value,index)
    for index,state in enumerate(facts['states']):
        targets={0,index}
        for name in state.get('properties',{}):targets.update(representatives[signatures[(state['id'],name)]].values())
        f=next(cases({'type':'copy_properties','source':state},sorted(targets)))
        yield f
    for source in sources:
        for f in cases({'type':'copy_properties','source':source},repeats=3,wide=True):
            yield f
            yield dict(f,fail=0)
            yield dict(f,fail=1)
    for block,property_name in sorted(integers):
        values=sorted(map(int,domains[(block,property_name)]))
        for value in [values[0],values[-1],values[0]-1,values[-1]+1,{'type':'uniform','min_inclusive':values[0]-1,'max_inclusive':values[-1]+1}]:
            source={'type':'weighted','entries':[{'data':{'id':block},'weight':1},{'data':'stone','weight':1},{'data':'water','weight':1}]}
            yield from cases({'type':'randomized_int','source':source,'property':property_name,'values':value},repeats=3,wide=True)
    for name in ['missing','axis','level','age','power','distance','',None,False]:
        for value in [0,1,-1,{'type':'uniform','min_inclusive':0,'max_inclusive':31}]:
            for f in cases({'type':'randomized_int','source':{'type':'random_block','blocks':[]},'property':name,'values':value},list(facts['defaults'].values()),wide=True):
                yield f
                yield dict(f,fail=0)
    for depth in [1,2,8,32]:
        source={'type':'random_block','blocks':[]}
        for i in range(depth):source={'type':'copy_properties','source':{'type':'randomized_int','source':source,'property':'level','values':{'type':'uniform','min_inclusive':0,'max_inclusive':15}}}
        yield from cases(source,wide=True)
    positions=[[0,0,0],[1,1,1],[-1,-64,-1],[17,-63,-33],[30000000,319,-30000000],[2147483647,-2147483648,2147483647],[16777217,65,-16777217]]
    rng=random.Random(7581)
    positions += [[rng.randrange(-30000000,30000000),rng.randrange(-2032,2032),rng.randrange(-30000000,30000000)] for _ in range(9)]
    def noise_cases(definition,wide=True):
        for f in cases(definition,[0]*len(positions),wide=wide):
            yield dict(f,positions=positions)
    parameters=[{'base_octave':-3,'octave_count':3},{'base_octave':-32},{'base_octave':32},{'base_octave':0,'base_amplitude':1000,'normalize':False},{'base_octave':-1,'octave_count':2,'amplitude_modifiers':[0,0]}]
    for kind in ['noise','noise_threshold','dual_noise']:
        for noise in parameters:
            for scale in [0.01,0.5,1.0,1e-45,3.4028234663852886e38]:
                for seed in [0,-1,9223372036854775807]:
                    f={'type':kind,'seed':seed,'noise':noise,'scale':scale}
                    if kind=='noise_threshold':f.update(threshold=0.0,high_chance=0.5,default_state='stone',low_states=['water','dirt'],high_states=['oak_log','poppy','piston'])
                    else:f['states']=['stone','water','dirt','oak_log','poppy','piston','furnace']
                    if kind=='dual_noise':f.update(variety=[1,5],slow_noise={'base_octave':-4,'octave_count':2},slow_scale=scale)
                    yield from noise_cases(f)
    template={'type':'noise','seed':0,'noise':{'base_octave':0},'scale':0.1,'states':['stone','water','oak_log']}
    for seed in [18446744073709551617,-18446744073709551617,1e100,-1e100,1.9,-1.9,0.9,-0.9,True,None,'1']:
        yield from noise_cases(dict(template,seed=seed))
    for scale in [0,-0.0,-1,1e-50,1e100,None,True,'0.1']:
        yield from noise_cases(dict(template,scale=scale),False)
    for noise in [{'base_octave':33},{'base_octave':-33},{'base_octave':0,'normalize':'legacy'},{'base_octave':0,'octave_count':0},None,False]:
        yield from noise_cases(dict(template,noise=noise),False)
    for states in [[],['missing'],None,False,'stone',[{'id':'water','properties':{'level':'1'}}]]:
        yield from noise_cases(dict(template,states=states),False)
    for variety in [1,64,[1,1],[64,64],[1,64],{'min_inclusive':2,'max_inclusive':7},[0,2],[2,1],[1,65],[],[1],False,None]:
        yield from noise_cases(dict(template,type='dual_noise',variety=variety,slow_noise={'base_octave':0,'normalize':False,'base_amplitude':1000},slow_scale=0.25),False)
    for threshold in [-1,-0.0,0,1,-1.1,1.1,None]:
        for chance in [-0.0,0,0.5,1,2,None]:
            yield from noise_cases(dict(template,type='noise_threshold',threshold=threshold,high_chance=chance,default_state='stone',low_states=['water'],high_states=['poppy']),False)
    for definition in (provider_resources or {}).values():yield from noise_cases(definition)
    true={'type':'true'}
    false={'type':'not','predicate':true}
    inside={'type':'inside_world_bounds'}
    predicates=[true,false,inside,{'type':'all_of','predicates':[]},{'type':'any_of','predicates':[]},
        {'type':'all_of','predicates':[false,inside]}, {'type':'all_of','predicates':[inside,false]},
        {'type':'any_of','predicates':[true,inside]}, {'type':'any_of','predicates':[inside,true]}]
    predicates += [{'type':'inside_world_bounds','offset':[0,y,0]} for y in [-2147483648,-16,-1,1,16,17]]
    empty={'type':'random_block','blocks':[]}
    choices=[empty,{'id':'water'}, {'type':'weighted','entries':entries([1,2,3])},
        {'type':'copy_properties','source':empty}, template,
        {'type':'randomized_int','source':{'id':'water'},'property':'level','values':{'type':'uniform','min_inclusive':0,'max_inclusive':16}}]
    positions=[[17,y,-33] for y in [-2147483648,-65,-64,-63,0,318,319,320,2147483647]]
    for predicate in predicates:
        for choice in choices:
            for fallback in [None,empty,{'id':'oak_log'}]:
                definition={'type':'rule_based','fallback':fallback,'rules':[
                    {'if_true':predicate,'then':choice}, {'if_true':inside,'then':empty},
                    {'if_true':false,'then':{'type':'copy_properties','source':empty}}]}
                for f in cases(definition,[0]*len(positions),wide=True):
                    f=dict(f,positions=positions)
                    yield f
                    yield dict(f,query_fail=0)
                    yield dict(f,query_fail=2)
                    yield dict(f,fail=0)
    for rules in [[],None,False,[{}],[{'if_true':true}],[{'then':empty}],
        [{'if_true':false,'then':{'type':'missing'}}], [{'if_true':{'type':'missing'},'then':empty}]]:
        for fallback in [None,empty,{'id':'stone'},'stone',{'type':'missing'}]:
            yield from cases({'type':'rule_based','rules':rules,'fallback':fallback},wide=True)
    for depth in [1,2,8,32]:
        source=empty
        for _ in range(depth):source={'type':'rule_based','fallback':source,'rules':[{'if_true':true,'then':empty}]}
        yield from cases(source,wide=True)
    def predicate_cases(predicate, states=None, wide=True):
        if states is None:states=[0]*len(positions)
        definition={'type':'rule_based','rules':[{'if_true':predicate,'then':{'id':'stone'}}],'fallback':empty}
        for f in cases(definition,states,wide=wide):
            f=dict(f,positions=positions)
            yield f
            yield dict(f,query_fail=1,fail=1)
    anchors=[{kind:v} for kind in ['absolute','above_bottom','below_top','relative_to_sea_level'] for v in [-2032,-64,0,63,2031]]
    for minimum in anchors:
        for maximum in anchors:
            yield from predicate_cases({'type':'height_range','min_inclusive':minimum,'max_inclusive':maximum},wide=False)
    for invalid in [None,1,{}, {'absolute':2032}, {'below_top':-2033}, {'absolute':0,'above_bottom':0}]:
        yield from predicate_cases({'type':'height_range','min_inclusive':invalid,'max_inclusive':{'absolute':0}},wide=False)
    for tag in [*sorted(tags),'missing',':','#minecraft:logs','UPPERCASE',None,False]:
        yield from predicate_cases({'type':'matching_block_tag','tag':tag},list(facts['defaults'].values()),wide=False)
    for name in facts['defaults']:
        matching=[i for i,state in enumerate(facts['states']) if state['id']==name]
        states=sorted(set(matching+[0,len(facts['states'])-1]))
        for blocks in [name,[name]]:
            yield from predicate_cases({'type':'matching_blocks','blocks':blocks},states,wide=False)
    for blocks in [*['#'+name for name in sorted(tags)], [], ['stone','water','stone'], '#missing', ['#minecraft:logs'], ['missing'], ['stone',None], None, False, {}]:
        yield from predicate_cases({'type':'matching_blocks','blocks':blocks},list(facts['defaults'].values()),wide=False)
    for fluids in ['empty','flowing_water','water','flowing_lava','lava',*['#'+name for name in sorted(fluid_tags or {})],[],['water','water','flowing_lava'],'missing','#missing',['#minecraft:water'],False,None]:
        yield from predicate_cases({'type':'matching_fluids','fluids':fluids},all_states,wide=False)
    biome_indices=list(range(len(biomes or {'minecraft:plains':{}})))
    for selected in [*sorted(biomes or {}),*['#'+name for name in sorted(biome_tags or {})],[],['plains','desert','plains'],'plains','missing','#missing',['#minecraft:is_badlands'],False,None]:
        predicate={'type':'matching_biomes','biomes':selected}
        for condition in [predicate,{'type':'not','predicate':predicate}, {'type':'volume_match','min':[0,0,0],'max':[1,0,1],'match':predicate}]:
            for f in predicate_cases(condition,[0]*len(biome_indices),wide=False):yield dict(f,biomes=biome_indices)
    for selected in ['plains','#horizon:ordered',[]]:
        for f in predicate_cases({'type':'any_of','predicates':[{'type':'matching_biomes','biomes':selected},inside]},[0]*len(biome_indices)):
            yield dict(f,biomes=biome_indices)
    for offset in [None,[0,0,0],[1,-1,16],[17,18,-19],[2147483647,-2147483648,2147483647],[4294967297,0,0],[1.9,-1.9,0],[],[1],[1,2,3,4],False,{}]:
        predicate={'type':'unobstructed','offset':offset}
        for condition in [predicate,{'type':'not','predicate':predicate},{'type':'volume_match','min':[-1,0,-1],'max':[1,0,1],'match':predicate}]:
            for f in predicate_cases(condition):
                yield dict(f,unobstructed=True)
                yield dict(f,unobstructed=False)
    for offset in [[0,0,0],[16,-16,1],[-2147483648,0,-2147483648],[17,0,0],[],None]:
        yield from predicate_cases({'type':'matching_block_tag','tag':'minecraft:logs','offset':offset})
    for minimum,maximum in [([0,0,0],[0,0,0]),([-1,-1,-1],[1,1,1]),([-16,0,0],[16,1,1]),([1,0,0],[0,0,0]),([-17,0,0],[0,0,0]),([-2147483648,0,0],[-2147483648,0,0])]:
        for child in [true,false,inside,{'type':'height_range','min_inclusive':{'above_bottom':0},'max_inclusive':{'below_top':0}}, {'type':'matching_block_tag','tag':'minecraft:logs'}, {'type':'not','predicate':{'type':'matching_block_tag','tag':'minecraft:logs'}}]:
            yield from predicate_cases({'type':'volume_match','min':minimum,'max':maximum,'match':child})
    for depth in [1,2,3]:
        child={'type':'not','predicate':{'type':'matching_block_tag','tag':'minecraft:logs'}}
        for _ in range(depth):child={'type':'volume_match','min':[0,0,0],'max':[1,1,1],'match':child}
        yield from predicate_cases(child)
    for definition in (provider_resources or {}).values():
        if definition.get('type') == 'minecraft:rule_based':
            yield from cases(definition,all_states,wide=True)
    for bounds in [[0,1,0],[2147483647,2,0],[-2147483648,384,63]]:
        for f in cases({'type':'rule_based','rules':[{'if_true':inside,'then':{'id':'stone'}}]},[0]*len(positions),wide=True):
            yield dict(f,bounds=bounds,positions=positions)
    for kind in ['solid','replaceable']:
        for offset in [None,[0,0,0],[16,-16,1],[-2147483648,0,0],[17,0,0],[],[0,0],False,[1.9,0,0]]:
            predicate={'type':kind,'offset':offset}
            definition={'type':'rule_based','fallback':{'id':'water'},'rules':[{'if_true':predicate,'then':{'id':'stone'}}]}
            for f in cases(definition,all_states,wide=True):yield dict(f,positions=positions)
            for f in cases(definition,[0,1],wide=True):
                yield dict(f,fail=0,positions=positions)
                yield dict(f,fail=1,positions=positions)
        for condition in [{'type':'all_of','predicates':[false,{'type':kind}]},{'type':'any_of','predicates':[true,{'type':kind}]},{'type':'not','predicate':{'type':kind}},{'type':'volume_match','min':[-1,0,-1],'max':[1,0,1],'match':{'type':kind}}]:
            definition={'type':'rule_based','fallback':empty,'rules':[{'if_true':condition,'then':empty},{'if_true':{'type':kind},'then':{'id':'stone'}}]}
            for f in cases(definition,list(facts['defaults'].values()),wide=True):
                yield f
                yield dict(f,fail=0)
                yield dict(f,fail=2)
    if survival_rules:
        representatives={}
        neighbors=[facts['defaults'][name] for name in ['minecraft:air','minecraft:stone','minecraft:pale_moss_carpet','minecraft:shulker_box','minecraft:moving_piston'] if name in facts['defaults']]
        def survival_provider(condition):
            return {'type':'rule_based','fallback':{'id':'water'},'rules':[{'if_true':condition,'then':{'id':'stone'}}]}
        for state in facts['states']:
            rule=survival_rules.get(state['id'])
            if rule is None:continue
            key=survival.behavior_key(rule,state)
            representatives.setdefault(key,state)
            yield from cases(survival_provider({'type':'would_survive','state':state}),neighbors)
        for state in representatives.values():
            yield from cases(survival_provider({'type':'would_survive','state':state}),all_states)
            if survival_rules[state['id']] in ['Crop','PitcherCrop','Mushroom']:
                for brightness in [-2147483648,-1,0,7,8,9,12,13,14,15,2147483647]:
                    for f in cases(survival_provider({'type':'would_survive','state':state}),all_states[::7],wide=True):
                        yield dict(f,brightness=brightness)
                        yield dict(f,brightness=brightness,query_fail=0)
                        yield dict(f,brightness=brightness,fail=0)
            for offset in [[-16,0,16],[16,-16,0],[17,0,0],[-17,0,0],[0,0],None,'bad']:
                condition={'type':'would_survive','state':state,'offset':offset}
                for f in cases(survival_provider(condition),neighbors,wide=True):
                    yield dict(f,positions=positions)
                    yield dict(f,positions=positions,fail=0)
                    yield dict(f,positions=positions,query_fail=0)
            condition={'type':'would_survive','state':state}
            for combined in [{'type':'all_of','predicates':[false,condition]},{'type':'any_of','predicates':[true,condition]},{'type':'not','predicate':condition},{'type':'volume_match','min':[-1,0,-1],'max':[1,0,1],'match':condition}]:
                definition={'type':'rule_based','rules':[{'if_true':combined,'then':empty},{'if_true':condition,'then':{'id':'stone'}}]}
                for f in cases(definition,neighbors,wide=True):
                    yield f
                    yield dict(f,fail=1)
        for state in ['stone','minecraft:water',{'id':'ladder','properties':{'facing':'east'}},{'id':'water','properties':{'level':'+01'}},'missing',{},None,False,[],{'id':'pale_moss_carpet','properties':{'bottom':'bad'}}]:
            yield from cases(survival_provider({'type':'would_survive','state':state}),neighbors,wide=True)
        yield from cases(survival_provider({'type':'would_survive'}),neighbors)
    support_states = all_states if support_states is None else support_states
    support_ids=set(support_states)
    dynamic_support_states=[state for state in all_states if state not in support_ids]
    for direction in ['down','up','north','south','west','east','UP','missing',None,False,1]:
        for offset in [None,[0,0,0],[16,-16,1],[-2147483648,0,0],[17,0,0],[],[1.9,0,0]]:
            condition={'type':'has_sturdy_face','direction':direction,'offset':offset}
            definition={'type':'rule_based','fallback':{'id':'water'},'rules':[{'if_true':condition,'then':{'id':'stone'}}]}
            valid=direction in ['down','up','north','south','west','east'] and offset not in [[17,0,0],[]]
            for f in cases(definition,all_states if valid else [0],wide=False):yield dict(f,positions=positions)
            for f in cases(definition,[0,1],wide=True):yield dict(f,positions=positions,fail=0)
    for direction in ['down','up','north','south','west','east']:
        definition={'type':'rule_based','fallback':{'id':'water'},'rules':[{'if_true':{'type':'has_sturdy_face','direction':direction},'then':{'id':'stone'}}]}
        if dynamic_support_states:
            for f in cases(definition,dynamic_support_states,wide=True):
                for failure in [0,1,2]:yield dict(f,query_fail=failure,positions=positions)
        for condition in [{'type':'all_of','predicates':[false,{'type':'has_sturdy_face','direction':direction}]},{'type':'any_of','predicates':[true,{'type':'has_sturdy_face','direction':direction}]},{'type':'not','predicate':{'type':'has_sturdy_face','direction':direction}}]:
            definition={'type':'rule_based','rules':[{'if_true':condition,'then':empty},{'if_true':{'type':'has_sturdy_face','direction':direction},'then':{'id':'stone'}}]}
            for f in cases(definition,support_states[::17],wide=True):
                yield f
                yield dict(f,fail=1)
    for name in [*(catalog_resources or {}),'horizon:unknown','minecraft:stone','STONE',':']:
        for f in cases(name,repeats=2,wide=True):
            yield f
            yield dict(f,holder=True)
        for wrapper in [{'type':'copy_properties','source':name},{'type':'rotated','state':name},
            {'type':'randomized_int','source':name,'property':'age','values':1},
            {'type':'rule_based','fallback':name,'rules':[]},
            {'type':'rule_based','rules':[{'if_true':true,'then':name}]}]:
            for f in cases(wrapper,repeats=2,wide=True):
                yield f
                yield dict(f,holder=True,fail=0)
        for f in cases({'id':'stone','type':'copy_properties','source':name},wide=True):
            yield f
            yield dict(f,holder=True)
    for definition in (provider_resources or {}).values():
        for f in cases(definition,list(facts['defaults'].values()),wide=True):yield dict(f,holder=True)
    for value in [None,False,1,[],{}, {'type':'unknown'}, {'id':'missing'}, {'type':False}, {'type':'simple'}, {'type':'random_block'}, {'type':'weighted'}]:
        yield from cases(value)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare',action='store_true')
    parser.add_argument('--resume',action='store_true',help='Reuse stages only after verifying input, executable and output hashes')
    args=parser.parse_args()
    build=lab.LOCAL/'rust-state-provider-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    checkpoint_path=build/'run-state.json'
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStateProviderOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaBlockStateCodecOracle','VanillaHeightmapOracle','VanillaMaterialConditionOracle','VanillaBiomeTemperatureOracle','VanillaAnchorOracle','VanillaBlockPropertiesOracle','VanillaBlockSupportOracle','VanillaBlockSurvivalOracle','VanillaCollisionOracle']]
    sources=[oracle,*helpers,lab.ROOT/'tools/test_block_survival_parity.py',lab.ROOT/'tools/test_biome_temperature_parity.py',lab.ROOT/'tools/test_material_condition_parity.py',lab.ROOT/'tools/test_worldgen_parity.py',Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/state_provider_fixture.rs']
    tested_sources=base.source_hashes(sources)
    environment={'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'dependencies':{str(path):lab.digest(path,'sha256') for path in [*jars.values(),*libraries]}}
    checkpoint=survival.load_checkpoint(checkpoint_path,tested_sources,environment) if args.resume else None
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,stdout=log,stderr=subprocess.STDOUT,timeout=3600)
    dependency_path=os.pathsep.join(map(str,libraries))
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaStateProviderOracle',mode,path],side+'-'+mode)
    if checkpoint is None:
        checkpoint_path.unlink(missing_ok=True)
        tags=worldgen.tag_resources(jars['server'])
        if not tags or tags!=worldgen.tag_resources(jars['client']):raise ValueError('client/server tags differ')
        tags.update({'horizon:empty':json.dumps({'values':[]}), 'horizon:ordered':json.dumps({'values':['minecraft:water','minecraft:stone','minecraft:water']})})
        (build/'tags.json').write_text(json.dumps(tags,separators=(',',':')),encoding='utf-8')
        def fluid_tag_resources(jar):
            with zipfile.ZipFile(jar) as archive:
                prefix='data/minecraft/tags/fluid/'
                return {'minecraft:'+name[len(prefix):-5]:archive.read(name).decode('utf-8') for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith('.json')}
        fluid_tags=fluid_tag_resources(jars['server'])
        if fluid_tags!=fluid_tag_resources(jars['client']):raise ValueError('client/server fluid tags differ')
        fluid_tags['horizon:empty']='{"values":[]}'
        (build/'fluid-tags.json').write_text(json.dumps(fluid_tags,separators=(',',':')),encoding='utf-8')
        biomes=temperature.biome_resources(jars['server']);biome_tags=material.biome_tag_resources(jars['server'])
        if biomes!=temperature.biome_resources(jars['client']) or biome_tags!=material.biome_tag_resources(jars['client']):raise ValueError('client/server biome resources differ')
        biome_tags['horizon:empty']='{"values":[]}'
        biome_tags['horizon:ordered']='{"values":["minecraft:plains","minecraft:desert","minecraft:plains"]}'
        for name,data in [('biomes',biomes),('biome_tags',biome_tags)]:
            (build/(name+'.json')).write_text(json.dumps(data,separators=(',',':')),encoding='utf-8')
        run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars['server']),dependency_path]),'dev.horizonlab.parity.VanillaBlockPropertiesOracle','export',build],'properties-export')
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars['server']),dependency_path]),'dev.horizonlab.parity.VanillaBlockSupportOracle','export',build],'support-export')
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars['server']),dependency_path]),'dev.horizonlab.parity.VanillaBlockSurvivalOracle','definitions',build],'survival-export')
        survival_classes=json.loads((build/'survival-classes.json').read_text())
        survival_rules=survival.bind_definitions(survival_classes,json.loads((build/'survival-placement-classes.json').read_text()),json.loads((build/'survival-support-tags.json').read_text()))
        (build/'survival-rules.json').write_text(json.dumps(survival_rules,separators=(',',':')))
        support_states=json.loads((build/'support-cached-states.json').read_text())
        if len(support_states)!=35524:raise ValueError('incomplete support state cache')
        for side in jars:java(side,'export',build/f'facts-{side}.json')
        facts=json.loads((build/'facts-server.json').read_text())
        if facts!=json.loads((build/'facts-client.json').read_text()):raise ValueError('client/server registry metadata differs')
        if len(facts['states'])!=35723:raise ValueError('incomplete pinned state registry')
        def resources(jar):
            with zipfile.ZipFile(jar) as archive:
                return {name:json.loads(archive.read(name)) for name in sorted(archive.namelist()) if name.startswith('data/minecraft/worldgen/block_state_provider/') and name.endswith('.json')}
        provider_resources=resources(jars['server'])
        if provider_resources!=resources(jars['client']):raise ValueError('bundled noise provider resources differ')
        (build/'providers.json').write_text(json.dumps(provider_resources,separators=(',',':')),encoding='utf-8')
        catalog_resources={name.removeprefix('data/minecraft/worldgen/block_state_provider/').removesuffix('.json'):json.dumps(value,separators=(',',':')) for name,value in provider_resources.items()}
        catalog_resources={'minecraft:'+name:value for name,value in catalog_resources.items()}
        catalog_resources['horizon:empty']=json.dumps({'type':'random_block','blocks':[]})
        catalog_resources['horizon:tagged']=json.dumps({'type':'rule_based','rules':[{'if_true':{'type':'matching_block_tag','tag':'minecraft:logs'},'then':'minecraft:cave_vines_head'}],'fallback':'horizon:empty'})
        for depth in range(12):
            catalog_resources['horizon:chain_'+str(depth)]=json.dumps({'type':'copy_properties','source':'horizon:chain_'+str(depth-1) if depth else 'horizon:tagged'})
        (build/'catalog.json').write_text(json.dumps(catalog_resources,separators=(',',':')),encoding='utf-8')
        inputs=build/'inputs.txt';fixture_count=0;requested_samples=0
        with inputs.open('w',encoding='utf-8') as output:
            for fixture in corpus(facts,tags,provider_resources,catalog_resources,fluid_tags,biomes,biome_tags,support_states,survival_rules):
                output.write(json.dumps(fixture,separators=(',',':'))+'\n')
                fixture_count+=1;requested_samples+=len(fixture['states'])*fixture['repeats']*2
        fixture_paths=[inputs,build/'survival-extra-definitions.json',build/'survival-placement-classes.json',build/'survival-support-tags.json',build/'survival-classes.json',build/'survival-rules.json',build/'cached-support.nbt',build/'support-cached-states.json',build/'block-properties.nbt',build/'cached-collision.nbt',build/'biomes.json',build/'biome_tags.json',build/'fluid-tags.json',build/'catalog.json',build/'providers.json',build/'tags.json',build/'facts-client.json',build/'facts-server.json'];tested_fixtures=base.source_hashes(fixture_paths)
        run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
        metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
        binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
        counts={'block_states':len(facts['states']),'blocks':len(facts['defaults']),'integer_properties':len(facts['integers']),'codec_inputs':fixture_count,'provider_types':10,'named_providers':len(catalog_resources),'rule_predicate_types':16,'survival_supported_blocks':len(survival_rules),'survival_supported_states':sum(s['id'] in survival_rules for s in facts['states']),'support_cached_states':len(support_states),'support_dynamic_states':len(facts['states'])-len(support_states),'biomes':len(biomes),'biome_tag_files':len(biome_tags),'fluid_tag_files':len(fluid_tags),'bundled_provider_definitions':len(provider_resources),'block_tag_files':len(tags),'requested_samples':requested_samples}
        base.ensure_sources_unchanged(sources,tested_sources)
        checkpoint={'schema':1,'environment':environment,'sources_sha256':tested_sources,'fixtures_sha256':{str(lab.ROOT/name):digest for name,digest in tested_fixtures.items()},'artifacts_sha256':{str(path):lab.digest(path,'sha256') for path in [binary,*sorted(classes.rglob('*.class'))]},'binary':str(binary),'counts':counts,'outputs':{}}
        survival.save_checkpoint(checkpoint_path,checkpoint)
        print('Verified inputs and compiled executables checkpointed',flush=True)
    def stage(name,action):
        output=build/(name+'.txt');previous=checkpoint['outputs'].get(name)
        if previous is None or not output.exists() or lab.digest(output,'sha256')!=previous:
            action()
            checkpoint['outputs'][name]=lab.digest(output,'sha256')
            survival.save_checkpoint(checkpoint_path,checkpoint)
            print(f'{name}: completed output checkpointed',flush=True)
    stage('rust',lambda:run([checkpoint['binary'],'state_provider_fixture',build],'rust'));references=[]
    for side,jar in jars.items():
        stage(side,lambda:java(side,side,build));reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} state-provider output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources)
    survival.load_checkpoint(checkpoint_path,tested_sources,environment)
    report={'schema':1,**{key:value for key,value in environment.items() if key!='dependencies'},**checkpoint['counts'],'sources_sha256':tested_sources,'fixtures_sha256':{str(Path(name).relative_to(lab.ROOT)):digest for name,digest in checkpoint['fixtures_sha256'].items()},'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BlockStateProvider DIRECT_CODEC/TYPED_CODEC and simple, weighted, random_block, rotated, copy_properties, randomized_int three noise-based providers and rule_based over the pinned state registry. All states and block defaults, full-state versus typed exclusive decoding, ordered holder sets/tags, duplicates, empty optional results versus world-state fallback, zero/overflow weights, query failures, every state across six fixed rotations, nested random rotations, property copying for every target state against representatives of every compatible property value, integer-property ranges/missing properties, composed transforms and subsequent random consumption. Native property equality families are checked for ambiguity; Rust validates Cartesian state tables and uses compact property strides. Both legacy/xoroshiro streams execute directly and through WorldgenRandom. Noise comparisons include every bundled provider definition, independent legacy seeding, float/double scaling, dual-noise variety selection, threshold draw order and coordinate/codec boundaries. Rule-based comparisons cover true/not/all_of/any_of/inside_world_bounds/height_range/volume_match/matching_block_tag/matching_blocks/matching_fluids/matching_biomes/unobstructed/solid/replaceable/has_sturdy_face/would_survive predicates, ordered optional results, fallbacks, nested rules and world-query failures. Every pinned block-state fluid identity, amount, falling/source flag and own-height bits are derived from descriptors and compared. Fluid-holder sets and all bundled fluid tags are tested against every state. Biome-holder membership and all bundled biome tags are compared across every pinned biome identity, including empty sets, missing names, ordered world queries and short-circuit composition. Unobstructed predicates compare null excluded entities, exact translated unit-shape bounds, ignored-but-validated offsets, true/false world results and failures. Named holder versus direct/typed decoding, nested named providers, missing names, inline ambiguity, ordered resource registration and all bundled named definitions are compared. Solidity and replacement predicates cover every block state, offset wrapping/validation, empty-child fallthrough, composed predicates and query failures using privately imported definition flags and Rust-calculated collision solidity. Has-sturdy-face predicates cover all 35,723 states through unified cached/dynamic support dispatch across six directions, native offset rules, failed state reads, block-entity query failures and ordered optional-child/boolean composition. Block entities are absent in this integration corpus; shulker animation-state shape selection is separately tested and active moving-piston shape generation is tested separately. Would-survive decodes candidate states and bounded offsets, evaluates all supported candidate states and each rule against every neighbor state, and preserves world read/entity failures and short-circuit/optional-child behavior. All pinned survival families are bound through the recorded definition inputs. Complete registry loading/reloading, full block placement behavior and full feature generation remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped state-provider parity passed: {report_path}')


if __name__=='__main__':main()
