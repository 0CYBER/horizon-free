#!/usr/bin/env python3
"""Compare moving-piston collision shapes and interpolation with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(facts):
    def bits(value):return struct.pack('>f',float(value)).hex()
    steps=[bits(v) for v in [-1,-0.0,0,0.25,0.5,0.75,1,2]]+['3e7fffff','3e800001','3f3fffff','3f400001']
    positions=[[0,0,0],[-17,-64,30000000],[2147483647,-2147483648,-2147483648]]
    partials=[bits(v) for v in [-1,0,0.5,1,2]]+['7fc00001','7f800000','ff800000']
    def fixture(state,index=0,**kwargs):
        result={'state':state,'position':positions[index%3],'direction':index%6,'extending':index%2==0,'source':False,'progress':steps[index%len(steps)],'previous':bits(0.25),'partial':partials[index%len(partials)],'noclip':-1,'fail':-1,'kinematic':False};result.update(kwargs);return result
    for state in range(len(facts['states'])):yield fixture(state,state)
    for state,value in enumerate(facts['states']):
        if value['id'] in ['minecraft:piston','minecraft:sticky_piston']:
            for direction in range(6):
                for extending in [False,True]:
                    for source in [False,True]:
                        for progress in steps:
                            for noclip in range(-1,6):yield fixture(state,direction,extending=extending,source=source,progress=progress,noclip=noclip)
        if value['id'].endswith('shulker_box') or value['id']=='minecraft:moving_piston':
            for extending in [False,True]:
                for direction in range(6):
                    for noclip in [-1,direction,direction^1]:
                        for fail in [-1,0]:yield fixture(state,direction,extending=extending,progress=bits(0.5),noclip=noclip,fail=fail)
    rng=random.Random(0x504953544f4e)
    stone=facts['defaults'].get('minecraft:stone',0)
    for index in range(10000):
        yield fixture(stone,index,kinematic=True,progress=f'{rng.getrandbits(32):08x}',previous=f'{rng.getrandbits(32):08x}',partial=f'{rng.getrandbits(32):08x}')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-piston-shape-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPistonShapeOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaFaceSupportOracle','VanillaBlockStateCodecOracle','VanillaCollisionOracle']]
    sources=[oracle,*helpers,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/piston_shape_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaPistonShapeOracle',mode,path],side+'-'+mode)
    java('server','export',build)
    facts=json.loads((build/'facts.json').read_text())
    if len(facts['states'])!=35723:raise ValueError('incomplete pinned registry')
    fixtures=list(corpus(facts));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'facts.json',build/'block-registry.nbt',build/'cached-collision.nbt'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'piston_shape_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} piston-shape output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'block_states':len(facts['states']),'kinematic_cases':sum(f['kinematic'] for f in fixtures),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual moving-piston entity collision shape, interpolation/offset float bits, movement direction and collision-related block-state selection. Every registered moved state is sampled, piston-base/source/extension/head thresholds and all noclip directions are crossed, and dynamic block-entity query failures are compared. Cached collision geometry is imported privately; uncached moved blocks use Rust support selection only in the tested empty-collision-context, absent-block-entity world. Piston ticking, entity pushes, live block entities, networking and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped piston-shape parity passed: {report_path}')


if __name__=='__main__':main()
