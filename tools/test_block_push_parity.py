#!/usr/bin/env python3
"""Compare entity pushes caused by replacement block collision geometry with vanilla."""
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


def corpus():
    def bits(v):return struct.pack('>d',float(v)).hex()
    def row(old,new,position=(0,0,0),entities=(),mutate=False,fail=-1):return dict(old=[list(map(bits,b)) for b in old],new=[list(map(bits,b)) for b in new],position=position,entities=[list(map(bits,b)) for b in entities],mutate=mutate,fail=fail)
    shapes=[[],[[0,0,0,1,1,1]],[[0,0,0,1,.9375,1]],[[0,0,0,1,.5,1]],[[0,.5,0,1,1,1]],[[0,0,0,.1,1,1]],[[0,0,0,1,1e-8,1]],[[0,0,0,1,1e-7,1]],[[-.5,-.5,-.5,1.5,1.5,1.5]],[[0,0,0,.25,.5,1],[.75,0,0,1,1,1]],[[0,0,0,0,0,0]]]
    for old in shapes:
        for new in shapes:
            for p in [(0,0,0),(17,-64,-33),(2147483647,0,-2147483648)]:
                entities=[[p[0]+.2,p[1]+y,p[2]+.2,p[0]+.8,p[1]+y+1.8,p[2]+.8] for y in [-1,0,.5,.9375,1,1.1]]
                for mutate in [False,True]:
                    for fail in [-1,0,1,2,3,5,8]:yield row(old,new,p,entities,mutate,fail)
    rng=random.Random(0x505553485550)
    for _ in range(5000):
        old=rng.choice(shapes);new=rng.choice(shapes);p=rng.choice([(0,0,0),(17,-64,-33)])
        entities=[]
        for _ in range(rng.randrange(8)):
            low=[rng.choice([-1,-0.0,0,.2,.5,1,1.5,float('nan')])+p[i] for i in range(3)];high=[v+rng.choice([0,1e-8,1e-7,.3,.6,1.8]) for v in low];entities.append([*low,*high])
        yield row(old,new,p,entities,bool(rng.randrange(2)),rng.choice([-1,-1,0,1,2,3,6,10]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-block-push-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockPushOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/block_push_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBlockPushOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'block_push_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} block-push output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Block.pushEntitiesUp with old/new collision shape queries and ordered selected entity lists. Compares unoptimized added-space geometry, world translation, entity query bounds, upward collision displacement, zero-distance teleports, current bounds after previous callbacks and partial state after provider failures. Empty/redundant shapes, partial shapes, large coordinates, degenerate/nonfinite entity boxes and callbacks moving later entities are covered. Collision shapes and entity selections are controlled snapshots; registered replacement dispatch, live entity indexing and actual teleport/network behavior remain separate integration work.'}
    lab.write_json(report_path,report);print(f'Scoped block push parity passed: {report_path}')


if __name__=='__main__':main()
