#!/usr/bin/env python3
"""Compare pinned vanilla compact/full block-state JSON codecs."""
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
    states=facts['states'];defaults=facts['defaults']
    for state in states:
        for full in [False,True]:yield [full,json.dumps(state,separators=(',',':'))]
    for name,index in sorted(defaults.items()):
        default=states[index]
        for value in [name,name.removeprefix('minecraft:'),':'+name.removeprefix('minecraft:'),{'id':name},{'id':name,'properties':default.get('properties',{})}]:
            for full in [False,True]:yield [full,json.dumps(value)]
        for properties in [None,False,0,1.5,'unknown',[],[{}],{}, {'unknown':'unknown'}]:
            for full in [False,True]:yield [full,json.dumps({'id':name,'properties':properties})]
        for key,original in default.get('properties',{}).items():
            for value in [None,True,False,0,1.0,[],{},'',original.upper(),'unknown',original]:
                changed=dict(default['properties']);changed[key]=value;changed['unknown']='ignored'
                for full in [False,True]:yield [full,json.dumps({'id':name,'properties':changed})]
    zeroes=[0x30,0x660,0x6f0,0x7c0,0x966,0x9e6,0xa66,0xae6,0xb66,0xbe6,0xc66,0xce6,0xd66,0xde6,0xe50,0xed0,0xf20,0x1040,0x1090,0x17e0,0x1810,0x1946,0x19d0,0x1a80,0x1a90,0x1b50,0x1bb0,0x1c40,0x1c50,0xa620,0xa8d0,0xa900,0xa9d0,0xa9f0,0xaa50,0xabf0,0xff10]
    for name,key in facts['integers']:
        for value in ['+0','-0','00','+01','-01','1.0','1e0',' 1','1 ','2147483647','2147483648','-2147483648','-2147483649','9'*80,'0'*80+'1','+','--1','+-1','\u00b2','\U0001d7d9',*[str(i) for i in range(-2,34)],*[chr(z)+chr(z+1) for z in zeroes]]:
            for full in [False,True]:yield [full,json.dumps({'id':name,'properties':{key:value}})]
    invalid=[None,False,1,[],{}, {'Name':'minecraft:stone'}, {'id':None}, {'id':True}, {'id':1}, {'id':[]}, {'id':{}},'minecraft:unknown','minecraft:',':','Minecraft:stone','minecraft:stone:foo','mine craft:stone',{'id':'minecraft:unknown','properties':{}}]
    for value in invalid:
        for full in [False,True]:yield [full,json.dumps(value)]
    for text in ['{"id":"minecraft:stone","id":"minecraft:water"}','{"id":"minecraft:water","properties":{"level":"0","level":"1"}}','{"id":"minecraft:water","properties":{"level":"1"},"properties":null}']:
        for full in [False,True]:yield [full,text]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-block-state-codec-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockStateCodecOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/block_state_codec_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBlockStateCodecOracle',mode,path],side+'-'+mode)
    for side in jars:java(side,'export',build/f'facts-{side}.json')
    facts=json.loads((build/'facts-server.json').read_text())
    if facts!=json.loads((build/'facts-client.json').read_text()):raise ValueError('client/server registry metadata differs')
    if len(facts['states'])!=35723:raise ValueError('incomplete pinned state registry')
    fixtures=list(corpus(facts));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'facts-client.json',build/'facts-server.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'block_state_codec_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} block-state codec output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'block_states':len(facts['states']),'blocks':len(facts['defaults']),'integer_properties':len(facts['integers']),'codec_inputs':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BlockState.CODEC and FULL_CODEC, every pinned runtime block state, compact/default versus full encoding, canonical object keys, namespaces, property omission/invalid fallback, unknown fields, malformed property containers, Java integer-property parsing including signs/leading zeroes/BMP decimal digits and overflow, invalid identifiers/types, duplicate fields. Registry identities, defaults and integer property kinds are imported from both authenticated JARs and must agree. This ports codec behavior over that metadata, not block gameplay. Native warning text, arbitrary malformed JSON/unpaired surrogate text, registry lifecycle/ownership and full material-rule codecs remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped block-state codec parity passed: {report_path}')


if __name__=='__main__':main()
