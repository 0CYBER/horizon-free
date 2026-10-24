#!/usr/bin/env python3
"""Compare entity collision shape selection and policy queries with pinned vanilla."""
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
    def box(v):return [struct.pack('>d',float(x)).hex() for x in v]
    def entity(bounds,collidable=True,spectator=False,policy=0,parent=-1):
        return dict(box=box(bounds),collidable=collidable,spectator=spectator,policy=policy,parent=parent)
    unit=[0,0,0,1,1,1]
    queries=[unit,[-0.0,-0.0,-0.0,0,0,0],[-1,-1,-1,2,2,2],[0,0,0,0,1,1],
             [0,0,0,float('nan'),1,1],[-float('inf'),-1,-1,float('inf'),2,2]]
    for width in [1e-8,math.nextafter(1e-7,0),1e-7,math.nextafter(1e-7,1),2e-7]:
        queries.extend([[0,0,0,width,width,width],[0,0,0,3*width,0,0]])
    boxes=[unit,[0.2,0.2,0.2,0.8,0.8,0.8],[1,0,0,2,1,1],[1+1e-7,0,0,2,1,1],
           [math.nextafter(1+1e-7,0),0,0,2,1,1],[0,0,0,1e-8,1,1],
           [0.5,0.5,0.5,0.5,0.75,0.75],[3,3,3,4,4,4]]
    for bounds in queries:
        for candidate in boxes:
            for collidable in [False,True]:
                for spectator in [False,True]:
                    for policy in range(3):
                        entities=[entity(unit,policy=policy),entity(candidate,collidable,spectator),entity(candidate,collidable,spectator,parent=0)]
                        for source in [-1,0,1]:
                            for fail in [-1,0,1,2,3,4]:yield dict(box=box(bounds),entities=entities,source=source,fail=fail)
    rng=random.Random(0x454e54434f4c4c)
    extents=[0,1e-8,1e-7,0.125,0.5,1.0,2.0]
    def random_box():
        low=[rng.choice([-1,-1e-8,0,0.5,1-1e-8,1,2]) for _ in range(3)]
        return [*low,*[v+rng.choice(extents) for v in low]]
    for index in range(3000):
        entities=[entity(random_box(),rng.choice([False,True]),rng.randrange(4)==0,rng.randrange(3),rng.randrange(-1,i) if i else -1) for i in range(rng.randrange(15))]
        yield dict(box=box(random_box()),entities=entities,source=rng.randrange(-1,len(entities)),fail=rng.choice([-1,-1,0,1,2,5,10]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-entity-collision-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityCollisionOracle.java'
    helper=oracle.with_name('VanillaFaceSupportOracle.java')
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/entity_collision_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEntityCollisionOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'entity_collision_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} entity-collision output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual EntityGetter.getEntityCollisions over privately allocated ordered entity snapshots. Compares average-size short circuits, epsilon-inflated query bounds, exact intersection/exclusion and spectator filtering, default canCollideWith including shared vehicle roots, overridden source policies, null-source canBeCollidedWith and emitted shape geometry. Policy callbacks are traced and can fail after each query; candidate lists and vehicle relationships are supplied snapshots. This does not establish live entity storage, broad-phase indexing, entity-type-specific collision policies, lifecycle or gameplay parity.'}
    lab.write_json(report_path,report);print(f'Scoped entity-collision parity passed: {report_path}')


if __name__=='__main__':main()
