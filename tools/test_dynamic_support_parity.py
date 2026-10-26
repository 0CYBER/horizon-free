#!/usr/bin/env python3
"""Compare dynamic block support shape selection with pinned vanilla."""
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


def corpus(facts,states):
    positions=[[0,0,0],[0,-2147483648,0],[0,2147483647,0],[17,-64,-33],[-1,319,-1],[30000000,63,-30000000],[2147483647,-2147483648,-2147483648]]
    rng=random.Random(0x44594e535550)
    positions += [[rng.randrange(-2147483648,2147483648),rng.randrange(-64,320),rng.randrange(-2147483648,2147483648)] for _ in range(48)]
    for state in states:
        name=facts['states'][state]['id']
        modes=['none','other','closed','opening','opened','closing'] if name.endswith('shulker_box') else ['none','other'] if name=='minecraft:moving_piston' else ['none']
        for position in positions:
            for entity in modes:
                yield {'state':state,'position':position,'entity':entity,'fail':-1}
        for entity in modes:
            for fail in [0,1,5,18]:yield {'state':state,'position':[17,-64,-33],'entity':entity,'fail':fail}

    directions=['down','up','north','south','west','east']
    targets=[facts.get('defaults',{}).get('minecraft:'+name) for name in ['stone','oak_slab','piston','sticky_piston','slime_block','piston_head']]
    for state in states:
        value=facts['states'][state]
        if value['id']!='minecraft:moving_piston':continue
        direction=directions.index(value.get('properties',{}).get('facing','north'))
        for moved in targets:
            if moved is None:continue
            for extending in [False,True]:
                for source in [False,True]:
                    for progress in [0,0.25,0.5,0.75,1]:
                        for noclip in [-1,direction,direction^1]:
                            piston={'state':moved,'direction':direction,'extending':extending,'source':source,'progress':struct.pack('>f',progress).hex(),'noclip':noclip}
                            f={'state':state,'position':[-17,-64,30000000],'entity':'piston','piston':piston,'fail':-1}
                            yield f
                            if progress==0.5 and noclip==-1:yield dict(f,fail=1)


def collision_corpus(facts,states):
    for fixture in corpus(facts,states):
        yield fixture
        yield dict(fixture,collision=True)
    shulkers=[state for state in states if facts['states'][state]['id']=='minecraft:shulker_box']
    rng=random.Random(0x5348554c4b4552)
    progress=[0.0,-0.0,1.0,0.1,0.2,0.3,0.5,0.7,0.9,struct.unpack('>f',bytes.fromhex('3f7fffff'))[0],struct.unpack('>f',bytes.fromhex('00000001'))[0]]
    progress_pairs=[(old,current) for old in progress for current in progress]
    progress_pairs.extend((rng.random(),rng.random()) for _ in range(2048))
    for state in shulkers:
        for old,current in progress_pairs:
            yield {'state':state,'position':[-17,2147483647,-33],'entity':'opened','fail':-1,'collision':True,'shulker_progress':[struct.pack('>f',v).hex() for v in [old,current]]}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-dynamic-support-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDynamicSupportOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaFaceSupportOracle','VanillaBlockStateCodecOracle','VanillaCollisionOracle']]
    sources=[oracle,*helpers,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/dynamic_support_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaDynamicSupportOracle',mode,path],side+'-'+mode)
    java('server','export',build)
    facts=json.loads((build/'facts.json').read_text());states=json.loads((build/'uncached-states.json').read_text())
    if len(states)!=199:raise ValueError('incomplete uncached support registry')
    fixtures=list(collision_corpus(facts,states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'facts.json',build/'uncached-states.json',build/'block-registry.nbt',build/'cached-collision.nbt'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'dynamic_support_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} dynamic-support output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'uncached_states':len(states),'collision_fixtures':sum(bool(f.get('collision')) for f in fixtures),'shulker_progress_fixtures':sum('shulker_progress' in f for f in fixtures),'active_piston_fixtures':sum(f['entity']=='piston' for f in fixtures),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual native support-shape and no-entity-context collision selection, block offsets and all support directions/types for the 199 uncached states. Bamboo, speleothem thickness/direction, scaffolding, powder snow and shulker support are compared over coordinate boundaries and privately allocated block-entity snapshots, including animation state, absent/wrong entity types and failures. Collision checks additionally cover exact shulker bounds and shape representation over progress interpolation boundaries and deterministic finite progress pairs. Moving-piston selection also delegates to actual native entity collision generation for active snapshots; Rust composes the independently tested piston algorithm with registry collision shapes across extension/retraction, source heads, progress boundaries, noclip directions and world-query failures. Entity lifecycle, ticking/pushing and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped dynamic-support parity passed: {report_path}')


if __name__=='__main__':main()
