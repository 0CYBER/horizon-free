#!/usr/bin/env python3
"""Compare bundled material-rule loading and evaluation with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
try:
    from . import lab,test_rust_parity as base,test_worldgen_parity as worldgen,test_material_condition_parity as material,test_biome_temperature_parity as biome
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_material_condition_parity as material
    import test_biome_temperature_parity as biome


def codec_inputs(rules):
    values=[json.loads(raw) for raw in rules.values()]
    bases=[{'type':'block','result_state':'minecraft:stone'}, {'type':'sequence','sequence':[]}, {'type':'condition','if_true':{'type':'hole'},'then_run':{'type':'bandlands'}}, {'type':'ore_vein','ore_block':'minecraft:copper_ore','raw_ore_block':'minecraft:raw_copper_block','filler_block':'minecraft:granite','raw_ore_chance':0.02,'density':1,'richness':0.5,'filler_gap':-1}]
    for base in bases:
        values += [base,dict(base,ignored=None)]
        for key in base:
            missing=base.copy();del missing[key];values.append(missing)
            for value in [None,[],False]:values.append(dict(base,**{key:value}))
    values += [None,False,0,[],{},'minecraft:overworld',{'type':'bandlands'},{'type':'bandlands','unused':False},{'type':'condition','if_true':{'type':'hole'},'then_run':'minecraft:unknown'}, {'type':'block','result_state':{'id':'minecraft:water','properties':{'level':'+01'}}}, {'type':'block','result_state':{'id':'minecraft:oak_log','properties':{'axis':'invalid'}}}]
    for chance in [-1,0,1,1.01,'0.02']:
        values.append(dict(bases[-1],raw_ore_chance=chance))
    for field in ['density','richness','filler_gap']:
        for value in ['minecraft:unknown',{'type':'minecraft:constant','argument':0},True,None]:values.append(dict(bases[-1],**{field:value}))
    return [json.dumps(value,separators=(',',':')) for value in values]


def corpus(rules,biomes):
    names=sorted(rules);biome_names=sorted(biomes)
    for legacy in [False,True]:
        for seed in [0,-1,87654321]:
            for case in range(2):
                ops=[]
                for i,name in enumerate(biome_names):
                    for j,y in enumerate([-64,-48,-32,-8,-1,0,16,32,48,63,64,80,120,200,319]):
                        x=[0,1,-1,15,16,17,2**24+3,-2**24-3][(i+j)%8];z=(i%17)-8
                        ops += [['biome',name],['input',[0,1,3,7][(i+j)%4],['0','3f000000','bf000000','3f800000'][(i+j)%4]],['xz',x,z,[-5,0,5][i%3],[-5,0,5][j%3]],['y',(j%6)+1,(i%8)+1,63 if j%2 else -2**31,y],['apply',x,y,z]]
                        if j%5==0:ops += [['apply',x+13,y-17,z+27]]
                fixture={'seed':seed,'legacy':legacy,'context':[-64,384,63],'conditions':[],'real_noise':True,'library_rules':names,'volume':[3,4,3,0,-64,0,2,1,2] if case==0 else [4,8,2,-17,24,7,1,2,1], 'possible_biomes':None if case==0 else biome_names,'ops':ops}
                if not legacy and seed==0 and case==0:fixture['library_json']=codec_inputs(rules)
                yield fixture


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-material-rule-library-parity';build.mkdir(parents=True,exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    bundle={}
    for field,kind in [('noises','noise'),('densities','density_function'),('rules','material_rule')]:
        resources=worldgen.registry_resources(jars['server'],kind)
        if not resources or resources!=worldgen.registry_resources(jars['client'],kind):raise ValueError('reference '+kind+' resources differ')
        prefix='data/minecraft/worldgen/'+kind+'/'
        bundle[field]={'minecraft:'+name[len(prefix):-5]:raw for name,raw in resources.items()}
    for field,getter in [('conditions',material.condition_resources),('biomes',biome.biome_resources),('biome_tags',material.biome_tag_resources)]:
        bundle[field]=getter(jars['server'])
        if not bundle[field] or bundle[field]!=getter(jars['client']):raise ValueError('reference '+field+' resources differ')
    counts={key:len(value) for key,value in bundle.items()}
    bundle['rules'].update({'horizon:a_forward':'{"type":"condition","if_true":{"type":"hole"},"then_run":"horizon:z_leaf"}','horizon:z_leaf':'{"type":"block","result_state":{"id":"minecraft:oak_log","properties":{"axis":"x"}}}','horizon:empty':'{"type":"sequence","sequence":[]}','horizon:constant_ore':'{"type":"ore_vein","ore_block":"minecraft:copper_ore","raw_ore_block":"minecraft:raw_copper_block","filler_block":"minecraft:granite","raw_ore_chance":0.02,"density":1,"richness":0.5,"filler_gap":-1}'})
    for field,value in bundle.items():(build/(field+'.json')).write_text(json.dumps(value,separators=(',',':')),encoding='utf-8')
    folder=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity'
    oracles=[folder/(name+'.java') for name in ['VanillaMaterialConditionOracle','VanillaAnchorOracle','VanillaBiomeTemperatureOracle','VanillaBlockStateCodecOracle']]
    sources=[*oracles,Path(__file__),*[lab.ROOT/'tools'/name for name in ['test_material_condition_parity.py','test_biome_temperature_parity.py','test_worldgen_parity.py','test_rust_parity.py','lab.py']],lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/material_condition_fixture.rs']
    tested_sources=base.source_hashes(sources);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,oracle,mode,path):run(['java','-Xmx1G','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.'+oracle,mode,path],side+'-'+mode)
    for side in jars:java(side,'VanillaBlockStateCodecOracle','export',build/f'facts-{side}.json')
    facts=json.loads((build/'facts-server.json').read_text())
    if facts!=json.loads((build/'facts-client.json').read_text()):raise ValueError('reference block metadata differs')
    fixtures=list(corpus(bundle['rules'],bundle['biomes']));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'facts-client.json',build/'facts-server.json',*[build/(field+'.json') for field in bundle]];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'material_condition_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,'VanillaMaterialConditionOracle',side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched});print(f'{side}: {matched:,} material-rule library output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'resources':counts,'fixtures':len(fixtures),'codec_inputs':sum(len(f.get('library_json',[])) for f in fixtures),'rule_applications':sum(sum(op[0]=='apply' for op in f['ops'])*len(f['library_rules']) for f in fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'All bundled material-rule definitions loaded against authenticated condition, biome/tag, block-state, noise and density registries, then compiled/evaluated for both world random modes. Forward/nested rule holders, all five rule kinds, malformed/omitted fields and property fallback, actual seeded density/richness/gap sources, strided chunk-volume caching, scalar fallback and independent context/application positions are covered. Surface depth and secondary noise remain supplied context inputs; preliminary density is zero for isolation. All bundled biome climates are used, with unrestricted or complete possible-biome sets. Rust library definitions are immutable and atomically loaded, with typed cycle/resource-bound errors. Rule/density encoding, live reload/registry owner validation, complete surface-stage chunks and gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped material-rule library parity passed: {report_path}')


if __name__=='__main__':main()
