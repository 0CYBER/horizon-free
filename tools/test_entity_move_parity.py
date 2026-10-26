#!/usr/bin/env python3
"""Compare Entity.move lifecycle ordering, state writes and recording with vanilla."""
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
    import itertools
    def bits(v):return struct.pack('>d',float(v)).hex()
    def single(v):return struct.pack('>f',float(v)).hex()
    def row(**kw):
        f=dict(movement=list(map(bits,[.4,-.2,.3])),position=list(map(bits,[.5,1.,.5])),velocity=list(map(bits,[.1,-.8,.2])),stuck=list(map(bits,[0,0,0])),piston_result=list(map(bits,[.25,0,0])),edge_result=list(map(bits,[.1,0,.2])),edge_override=False,no_physics=False,mover=0,client=False,client_authority=False,local_client=False,ground=True,horizontal=False,vertical=False,below=False,minor=False,minor_result=True,removed=False,distance=bits(5),clip_hit=False,fall_mutates=False,fall_removes=False,mutated_horizontal=False,mutated_vertical=False,mutated_below=False,emits=False,speed=single(.4),history=0,shapes=[],fail=-1)
        f.update(kw);return f
    scenes=[[],[[0,0,0,1,1,1]],[[1,0,0,2,3,1]],[[0,3,0,1,4,1]],[[0,0,1,1,3,2]],[[0,0,0,1,.9375,1]],[[0,0,0,1,1,1],[1,0,0,2,3,1],[0,0,1,1,3,2]]]
    motions=[[0,0,0],[-0.,0,0],[1e-200,-1e-200,1e-200],[1e-4,-1e-4,1e-4],[.4,-.2,.3],[2,-1,2],[-2,-5,-2],[0,9,0],[1e-5,0,-1e-5],[0,0,math.nan],[0,math.nan,0],[math.inf,0,0]]
    for shapes,motion,mover,client,authority,local in itertools.product(scenes,motions,range(5),[False,True],[False,True],[False,True]):
        yield row(shapes=[[bits(v) for v in b] for b in shapes],movement=list(map(bits,motion)),mover=mover,client=client,client_authority=authority,local_client=local,emits=True)
    for mover,no_physics,stuck,piston,fail in itertools.product(range(5),[False,True],[[0,0,0],[1e-200,0,0],[.1,.2,.3],[math.nan,1,1],[math.inf,0,0]],[[0,0,0],[-0.,0,0],[0,-0.,0],[0,0,-0.],[.25,0,0]],range(-1,12)):
        yield row(mover=mover,no_physics=no_physics,stuck=list(map(bits,stuck)),piston_result=list(map(bits,piston)),fail=fail,horizontal=True,vertical=True,below=True,minor=True)
    for shapes,flags,fail in itertools.product(scenes,range(32),range(-1,38)):
        yield row(shapes=[[bits(v) for v in b] for b in shapes],fall_mutates=True,fall_removes=bool(flags&1),mutated_horizontal=bool(flags&2),mutated_vertical=bool(flags&4),mutated_below=bool(flags&8),clip_hit=bool(flags&16),movement=list(map(bits,[2,-2,2])),fail=fail)
    for count,fail,no_physics in itertools.product([0,1,98,99,100],range(-1,30),[False,True]):yield row(history=count,fail=fail,no_physics=no_physics)
    rng=random.Random(0x4d4f56454d454e54)
    values=[-math.inf,-1e30,-2.,-0.,0.,1e-200,1e-7,1.,1e30,math.inf,math.nan]
    for _ in range(6000):
        yield row(movement=list(map(bits,rng.choice(motions))),shapes=[[bits(v) for v in b] for b in rng.choice(scenes)],mover=rng.randrange(5),stuck=[bits(rng.choice([0,.5,1,math.nan])) for _ in range(3)],piston_result=[bits(rng.choice(values)) for _ in range(3)],edge_override=bool(rng.randrange(2)),edge_result=[bits(rng.choice(values)) for _ in range(3)],velocity=[bits(rng.choice(values)) for _ in range(3)],no_physics=bool(rng.randrange(2)),client=bool(rng.randrange(2)),client_authority=bool(rng.randrange(2)),local_client=bool(rng.randrange(2)),ground=bool(rng.randrange(2)),horizontal=bool(rng.randrange(2)),vertical=bool(rng.randrange(2)),below=bool(rng.randrange(2)),minor=bool(rng.randrange(2)),minor_result=bool(rng.randrange(2)),removed=bool(rng.randrange(2)),distance=bits(rng.choice(values)),clip_hit=bool(rng.randrange(2)),fall_mutates=bool(rng.randrange(2)),fall_removes=bool(rng.randrange(2)),mutated_horizontal=bool(rng.randrange(2)),mutated_vertical=bool(rng.randrange(2)),mutated_below=bool(rng.randrange(2)),emits=bool(rng.randrange(2)),speed=single(rng.choice(values)),fail=rng.randrange(-1,34))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-entity-move-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityMoveOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/entity_move_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEntityMoveOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'entity_move_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} entity-move output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.move with native shape collision resolution (zero step height), native zero-restitution handling and final authority checks. Compares no-physics and piston early exits including signed zero, stuck multiplier reset, edge callback, fall-reset clip endpoints, movement recording cap, collision flag writes, ground/fall callback ordering, removal, passenger emission guard, speed multiplication and failures after partial state changes. Providers control piston/edge decisions, ground support, ray hits, landing effects and sound policy; live world integration and non-passenger movement emission remain separate work.'}
    lab.write_json(report_path,report);print(f'Scoped entity move parity passed: {report_path}')


if __name__=='__main__':main()
