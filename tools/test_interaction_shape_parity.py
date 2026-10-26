#!/usr/bin/env python3
"""Compare registered interaction shapes and ray-face overrides with vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
import math
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(info):
    def bits(v):return struct.pack('>d',float(v)).hex()
    shapes=[[],[[0,0,0,1,1,1]],[[.5,0,0,1,1,1]],[[0,.5,0,1,1,1]],[[0,0,0,.25,1,1],[.75,0,0,1,1,1]]]
    rays=[([-1,.5,.5],[2,.5,.5]),([-.5,-.25,.5],[1.5,1.75,.5]),([.5,.5,.5],[2,2,2]),([.5,2,.5],[.5,-1,.5])]
    def row(state,index,p=(0,0,0),shape=None):
        a,b=rays[index%len(rays)];return dict(state=state,position=p,shape=[[bits(v) for v in b] for b in (shape if shape is not None else shapes[1+index%4])],**{'from':[bits(v+p[i]) for i,v in enumerate(a)],'to':[bits(v+p[i]) for i,v in enumerate(b)]})
    for state in range(info['states']):
        for i in range(4):yield row(state,i,p=(0,0,0) if i%2==0 else (17,-64,-33))
    rng=random.Random(0x494e544552414354)
    for _ in range(10000):yield row(rng.randrange(info['states']),rng.randrange(4),p=rng.choice([(0,0,0),(17,-64,-33),(30000000,-64,-30000000)]),shape=rng.choice(shapes))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-interaction-shape-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaInteractionShapeOracle.java'
    helper=oracle.with_name('VanillaCollisionOracle.java')
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/interaction_shape_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaInteractionShapeOracle',mode,path],side+'-'+mode)
    java('server','export',build);info=json.loads((build/'info.json').read_text());fixtures=list(corpus(info));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'info.json',build/'block-registry.nbt',build/'interaction-shapes.nbt'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'interaction_shape_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} interaction-shape output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Complete registered interaction shape table with audited native implementation owners: default empty, cauldron, composter, hopper and scaffolding. Compares every state geometry, canonical/alias identity and actual BlockGetter.clipWithInteractionOverride with supplied collision shapes at multiple positions. World queries are rejected during native interaction-shape lookup. Imports reject wrong table kind, incomplete mapping, missing canonical identities, foreign state ordering and invalid state IDs. Outline/visual shape binding, complete ray-world binding and gameplay remain separate work.'}
    lab.write_json(report_path,report);print(f'Scoped interaction shape parity passed: {report_path}')


if __name__=='__main__':main()
