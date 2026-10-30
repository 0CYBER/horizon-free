#!/usr/bin/env python3
"""Compare contained fluid states, legacy blocks and height queries with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(facts):
    states=[i for i,state in enumerate(facts['states']) if state['id'] in ['minecraft:air','minecraft:water','minecraft:lava'] or ('copper_grate' in state['id'] and state.get('properties',{}).get('waterlogged')=='true')]
    for state in states:
        for above in states:
            for position in [[17,-64,-33],[2147483647,2147483647,-2147483648],[-2147483648,-2147483648,2147483647]]:
                for sturdy in [False,True]:
                    for fail in [-1,0,1,2]:
                        yield {'state':state,'above':above,'position':position,'sturdy':sturdy,'fail':fail}



def codec_corpus():
    import itertools
    for name in ['empty','water','flowing_water','lava','flowing_lava','missing','minecraft:water',':water','MINECRAFT:water','']:
        for definition in [name,{'id':name},{'id':name,'properties':None},{'id':name,'properties':[]},{'id':name,'properties':{'unknown':'x'}}]:
            yield definition
        for falling,level in itertools.product([None,True,False,0,1,'true','false','TRUE','False',{},[]],
                [None,True,False,0,1,8,9,'0','1','8','9','-1','+2','003','٢','３','2147483648','1.0',' 1',{},[]]):
            yield {'id':name,'properties':{'falling':falling,'level':level}}
    for definition in [None,True,0,[],{}, {'id':None}, {'id':3}, {'Name':'minecraft:water'}]:yield definition
    for name in ['water','flowing_water','lava','flowing_lava']:
        for falling in ['true','false']:
            for level in range(1,9):yield {'id':name,'properties':{'falling':falling,'level':str(level)}}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-fluid-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFluidOracle.java'
    helper=oracle.with_name('VanillaBlockStateCodecOracle.java')
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/fluid_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFluidOracle',mode,path],side+'-'+mode)
    for side in jars:java(side,'export',build/f'facts-{side}.json')
    facts=json.loads((build/'facts-server.json').read_text())
    if facts!=json.loads((build/'facts-client.json').read_text()):raise ValueError('reference block-state registries differ')
    if len(facts['states'])!=35723:raise ValueError('incomplete pinned block-state registry')
    fixtures=list(corpus(facts));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    codec_inputs=build/'codec-inputs.txt';codec_fixtures=list(codec_corpus());codec_inputs.write_text(''.join(json.dumps(v,separators=(',',':'))+'\n' for v in codec_fixtures),encoding='utf-8')
    fixture_paths=[inputs,codec_inputs,build/'facts-client.json',build/'facts-server.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'fluid_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} fluid output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'block_states':len(facts['states']),'fluid_kinds':5,'codec_fixtures':len(codec_fixtures),'fixtures':len(fixtures),'requested_height_queries':len(fixtures)*2,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'FluidState.CODEC decoding and encoding include all 37 states, object-only identifiers, lenient missing/invalid properties, Unicode integer levels and native defaults. Legacy conversion is checked for codec-derived fluid states as well. Native fluid identity, amount, falling/source/full flags, own-height bits and legacy block conversion for every pinned block state; all fluid-family identity pairs. Surface and camera height query order, above-fluid family checks, source-only downward-face support checks, coordinate wrapping and injected query failures are compared. The fixture supplies stone/air support states; general block support shapes, fluid spreading/ticks, entities and rendering remain outside this comparison.'}
    lab.write_json(report_path,report);print(f'Scoped fluid parity passed: {report_path}')


if __name__=='__main__':main()
