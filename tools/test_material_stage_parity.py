#!/usr/bin/env python3
"""Compare full surface-stage chunk application with pinned vanilla."""
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


def synthetic_rules():
    block=lambda name:{'type':'block','result_state':'minecraft:'+name}
    conditional=lambda condition,name:{'type':'condition','if_true':condition,'then_run':block(name)}
    rules={'horizon:water':block('water'),'horizon:air':block('air'),'horizon:stone':block('stone')}
    rules['horizon:probe']={'type':'sequence','sequence':[
        conditional({'type':'stone_depth','offset':0,'add_surface_depth':False,'secondary_depth_range':0,'surface_type':'ceiling'},'gravel'),
        conditional({'type':'steep'},'air'),
        conditional({'type':'water','offset':0,'surface_depth_multiplier':1,'add_stone_depth':True},'sand'),
        conditional({'type':'stone_depth','offset':1,'add_surface_depth':True,'secondary_depth_range':4,'surface_type':'floor'},'dirt'),block('stone')]}
    return {name:json.dumps(value,separators=(',',':')) for name,value in rules.items()}


def corpus(facts,biomes,settings=None,presets=None):
    defaults=facts['defaults'];all_biomes=sorted(biomes)
    scenes=[('overworld','stone',['minecraft:plains'],-64,256,63),('overworld','stone',['minecraft:eroded_badlands'],-64,256,63),('overworld','stone',['minecraft:frozen_ocean'],-64,256,63),('overworld','stone',['minecraft:deep_frozen_ocean','minecraft:eroded_badlands','minecraft:desert','minecraft:grove','minecraft:sulfur_caves'],-64,256,63),('overworld','stone',all_biomes,-64,256,63),('nether','netherrack',['minecraft:basalt_deltas','minecraft:crimson_forest','minecraft:soul_sand_valley','minecraft:nether_wastes','minecraft:warped_forest'],0,128,32),('end','end_stone',['minecraft:the_end','minecraft:end_highlands'],0,128,63)]
    def initial(minimum,height,sea,default,pattern):
        spans=[]
        for x in range(16):
            for z in range(16):
                top=minimum+height//2+(x*7+z*11)%27-13
                spans.append([x,minimum,z,top,default])
                if top<sea:spans.append([x,top+1,z,sea,defaults['minecraft:water']])
                if (x+z)%3==0:spans.append([x,minimum+16,z,minimum+20,defaults[['minecraft:air','minecraft:cave_air','minecraft:void_air'][(x+z)%9//3]]])
                if (x*3+z)%7==0:spans.append([x,top-3,z,top-2,defaults['minecraft:water']])
                if (x+z)%11==0:spans.append([x,top,z,top,defaults['minecraft:oak_log']])
                if (x+z)%13==0:spans.append([x,top+1,z,top+1,defaults['minecraft:oak_leaves']])
        if pattern%2:spans += [[0,minimum,z,minimum+height-1,defaults['minecraft:air']] for z in range(16)]
        return spans
    for scenario,(rule,default,choices,minimum,height,sea) in enumerate(scenes):
        for legacy in [False,True]:
            for seed in [0,-987654321]:
                cx,cz=[(0,0),(-1,2),(107,-59),(-300,911)][(scenario+int(legacy))%4]
                preliminary='minecraft:overworld/chunk_surface_level' if scenario%2==0 and rule=='overworld' else 0
                yield {'seed':seed,'legacy':legacy,'cx':cx,'cz':cz,'min':minimum,'height':height,'noise_min':minimum,'noise_height':height,'sea':sea,'write_min':minimum,'write_max':minimum+height-1,'default':defaults['minecraft:'+default],'rule':'minecraft:'+rule,'preliminary':preliminary,'biome_choices':choices,'possible':choices if legacy else None,'prime':seed==0,'initial':initial(minimum,height,sea,defaults['minecraft:'+default],scenario),'repeats':2 if scenario==3 else 1}
    for rule in ['horizon:water','horizon:air','horizon:probe']:
        for legacy in [False,True]:
            yield {'seed':1,'legacy':legacy,'cx':-1,'cz':1,'min':-64,'height':128,'noise_min':0,'noise_height':64,'sea':32,'write_min':-48,'write_max':47,'default':defaults['minecraft:stone'],'rule':rule,'preliminary':0,'biome_choices':['minecraft:plains'],'possible':None,'prime':False,'initial':initial(-64,128,32,defaults['minecraft:stone'],0),'repeats':2}
    for name in ['minecraft:plains','minecraft:eroded_badlands','minecraft:frozen_ocean']:
        yield {'seed':0,'legacy':False,'cx':0,'cz':0,'min':-64,'height':256,'noise_min':-64,'noise_height':256,'sea':63,'write_min':-64,'write_max':191,'default':defaults['minecraft:stone'],'rule':'minecraft:overworld','preliminary':0,'biome_choices':[name],'possible':[name],'prime':False,'initial':[],'repeats':2}


    # Fill through each bundled terrain router before applying its material rule.
    # Both stages share one compiler/context (and one native NoiseChunk).
    for setting,raw in sorted((settings or {}).items()):
        value=json.loads(raw);minimum=value['noise']['min_y'];height=value['noise']['height']
        choices=(['minecraft:nether_wastes','minecraft:basalt_deltas','minecraft:soul_sand_valley'] if setting=='minecraft:nether' else ['minecraft:the_end','minecraft:end_highlands'] if setting=='minecraft:end' else ['minecraft:plains','minecraft:eroded_badlands','minecraft:frozen_ocean','minecraft:grove','minecraft:desert'])
        for legacy in [False,True]:
            for seed,cx,cz in [(0,0,0),(-1,-2,3),(0x123456789abcdef,1875000,-1875000)]:
                fixture = {'seed':seed,'legacy':legacy,'cx':cx,'cz':cz,'min':minimum,'height':height,'noise_min':minimum,'noise_height':height,'sea':value['sea_level'],'write_min':minimum,'write_max':minimum+height-1,'default':defaults[value['default_block']],'fluid':defaults[value['default_fluid']],'rule':value['material_rule'],'router':value['noise_router'],'biome_zoom':True,'aquifers':value.get('aquifers'),'preliminary':value['noise_router']['chunk_surface_level'],'biome_choices':choices,'possible':choices if legacy else None,'prime':False,'initial':[],'repeats':2 if seed==0 else 1}
                if presets:
                    preset='minecraft:nether' if setting=='minecraft:nether' else 'minecraft:overworld'
                    source='end' if setting=='minecraft:end' else 'multi'
                    possible=(['minecraft:the_end','minecraft:end_highlands','minecraft:end_midlands','minecraft:small_end_islands','minecraft:end_barrens'] if source=='end' else sorted({entry['biome'] for entry in json.loads(presets[preset])}))
                    fixture.update(biome_source=source,biome_preset=preset,biome_source_json=({"type":"minecraft:the_end"} if source=="end" else {"type":"minecraft:multi_noise","preset":preset}),possible=possible if legacy else None)
                yield fixture


def top_queries(f):
    wrap=lambda value:((value+2**31)%2**32)-2**31
    return [[wrap((f['cx']<<4)+x),y,wrap((f['cz']<<4)+z),submerged]
            for x,z in [(0,0),(0,15),(15,0),(15,15),(1,7),(7,1),(7,7),(8,8)]
            for y in [f['min']-1,f['min'],f['sea']-1,f['sea'],f['min']+f['height']-1,f['min']+f['height']]
            for submerged in [False,True]]


def carving_fixture(f, facts):
    low,high=f['min']+1,f['min']+f['height']-8
    y=max(low+4,min(high-4,f['sea']))
    initial=[]
    defaults=facts['defaults']
    for x in range(8):
        initial.append([x,y,0,defaults['minecraft:grass_block' if x%2==0 else 'minecraft:mycelium']])
        for dy in range(1,4):initial.append([x,y-dy,0,defaults['minecraft:dirt']])
        if x%3==0:initial.append([x,y-1,0,defaults['minecraft:bedrock']])
    spans=[[x,0,low,high] for x in range(6)]+[[15,15,low,high],[8,8,low,high],
           [6,0,y,y],[6,0,y-3,y-2],[7,0,y-1,y+1]]
    return {'initial':initial,'spans':spans,'filters':[0,1]}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-material-stage-parity';build.mkdir(parents=True,exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts();bundle={}
    for field,kind in [('noises','noise'),('densities','density_function'),('rules','material_rule'),('settings','noise_settings'),('carvers','carver')]:
        values=worldgen.registry_resources(jars['server'],kind)
        if not values or values!=worldgen.registry_resources(jars['client'],kind):raise ValueError('reference '+kind+' resources differ')
        prefix='data/minecraft/worldgen/'+kind+'/';bundle[field]={'minecraft:'+name[len(prefix):-5]:raw for name,raw in values.items()}
    for field,getter in [('conditions',material.condition_resources),('biomes',biome.biome_resources),('biome_tags',material.biome_tag_resources),('block_tags',worldgen.tag_resources)]:
        bundle[field]=getter(jars['server'])
        if not bundle[field] or bundle[field]!=getter(jars['client']):raise ValueError('reference '+field+' resources differ')
    counts={key:len(value) for key,value in bundle.items()};bundle['rules'].update(synthetic_rules())
    for name,value in bundle.items():(build/(name+'.json')).write_text(json.dumps(value,separators=(',',':')),encoding='utf-8')
    folder=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity';oracles=[folder/(name+'.java') for name in ['VanillaMaterialStageOracle','VanillaMaterialConditionOracle','VanillaAnchorOracle','VanillaBiomeTemperatureOracle','VanillaHeightmapOracle','VanillaBiomeExport']]
    sources=[*oracles,Path(__file__),*[lab.ROOT/'tools'/name for name in ['test_material_condition_parity.py','test_biome_temperature_parity.py','test_worldgen_parity.py','test_rust_parity.py','lab.py']],lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/material_stage_fixture.rs']
    tested_sources=base.source_hashes(sources);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,libraries));run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,mode,path):run(['java','-Xmx1G','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.VanillaMaterialStageOracle',mode,path],side+'-'+mode)
    for side in jars:java(side,'export',build/f'facts-{side}.json')
    preset_exports=[]
    for side,jar in jars.items():
        path=build/f'presets-{side}.json';preset_exports.append(path)
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),'dev.horizonlab.parity.VanillaBiomeExport',path],side+'-presets')
        presets=json.loads(path.read_text())
        if not presets or ('presets' in bundle and presets!=bundle['presets']):raise ValueError('reference biome presets differ')
        bundle['presets']=presets
    (build/'presets.json').write_text(json.dumps(bundle['presets'],separators=(',',':')),encoding='utf-8')
    facts=json.loads((build/'facts-server.json').read_text())
    if facts!=json.loads((build/'facts-client.json').read_text()):raise ValueError('reference block metadata differs')
    fixtures=list(corpus(facts,bundle['biomes'],bundle['settings'],bundle['presets']))
    for f in fixtures:
        f['top_queries']=top_queries(f);f['carving']=carving_fixture(f,facts)
    inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    fixture_paths=[inputs,*preset_exports,build/'facts-client.json',build/'facts-server.json',*[build/(name+'.json') for name in bundle]];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust');metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,'material_stage_fixture',build],'rust')
    with (build/'rust.txt').open() as stream:
        if any((line.startswith('surface ') or line.startswith('top ') or line.startswith('carving ') or line.startswith('generatedcarving ')) and line.endswith(' error\n') for line in stream):raise AssertionError('valid surface fixture failed; inspect private Rust log')
    references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt');references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched});print(f'{side}: {matched:,} material-stage output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'resources':counts,'fixtures':len(fixtures),'generated_carving_passes':sum('biome_source' in f for f in fixtures),'carving_passes':sum(len(f['carving']['filters']) for f in fixtures),'top_material_queries':sum(len(f['top_queries']) for f in fixtures),'surface_builds':sum(f['repeats'] for f in fixtures),'terrain_builds':sum('router' in f for f in fixtures),'biome_chunks':sum(9 for f in fixtures if 'biome_source' in f),'biome_values':sum(9*f['height']*4 for f in fixtures if 'biome_source' in f),'block_values':sum((f['repeats']+int('router' in f)+len(f['carving']['filters'])+int('biome_source' in f))*f['height']*256 for f in fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual MaterialSystem.buildSurface and topMaterial against ProtoChunk before lighting, with complete raw sections, WG heightmap words, ordered fluid-postprocessing offsets and biome-lookup traces. All bundled rule registries, both seeded random modes, real surface/band/extension noises, real or supplied preliminary density, ore densities, empty/populated chunks, synthetic vertical spans/cavities/nondefault solids, clipped generation writes, lazy/primed maps and repeated surface builds sharing density contexts. Synthetic prefilled fixtures use a supplied deterministic block-position biome resolver; generated terrain fixtures first generate a 3x3 biome neighborhood through actual ChunkGenerator.doCreateBiomes, using pinned multi-noise preset tables or the End biome source. Complete stored biome values and subsequent BiomeManager block/quart lookup traces are compared. Both runtimes load the biome-source JSON against their biome/preset registries before generation. Rust imports independently authenticated preset tables; preset builders remain outside this comparison. Block air/fluid identities are imported from both authenticated JARs and motion predicates use resolved block tags. Each fixture applies an explicit carving mask twice through actual NoiseBasedChunkGenerator.applyCarvingMask after surface and top-material queries. The native aquifer is wrapped only to trace ordered computeSubstance/results/fluid-scheduling queries. Filtered and unfiltered passes, uncarvable tags, grass/mycelium/dirt restoration, repeated writes and disjoint runs are compared, followed by full chunk/heightmap/postprocessing dumps. Generated fixtures additionally invoke actual generateCarvers with biome carver settings selected from a 17x17 origin neighborhood, ordinal world seeds, native start checks and cave/canyon paths. The same source is reused after biome filling, preserving prior search state; a traced wrapper exposes uncached quart lookup order. Terrain/aquifer/postprocessing state is compared after procedural carving before explicit mask stress. Native carver settings retain actual carver holder lists while placed features are empty, as features are not evaluated in this stage. Each completed fixture also queries topMaterial at column edges and interior positions, above/below vertical bounds and around sea level, in dry and submerged contexts; these bind fresh one-cell material contexts without possible-biome restrictions. Empty blending only. Generated terrain fixtures also invoke actual NoiseBasedChunkGenerator.doFill before the surface stage, sharing a NoiseChunk and comparing full chunk state before and after surface builds for all bundled terrain routers and aquifers. Biome/feature/structure stages, nonempty blending, lighting and complete generated worlds/gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped material-stage parity passed: {report_path}')


if __name__=='__main__':main()
