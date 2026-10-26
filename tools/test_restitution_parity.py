#!/usr/bin/env python3
"""Compare post-collision restitution, bounce callbacks and partial failure state with vanilla."""
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
    def single(v):return struct.pack('>f',float(v)).hex()
    def row(flags=31,entity=0,block=1,vy=-2,gravity=(0.08,0.08),suppress=(False,False),tag=False,drag=0.98,fail=-1,mutate=False,dx=0.3,dz=-0.2,resolved=-0.1):
        return dict(x=bool(flags&1),z=bool(flags&2),vertical=bool(flags&4),below=bool(flags&8),living=bool(flags&16),sync=bool(flags&32),
                    entity=bits(entity),block=single(block),velocity=list(map(bits,[dx,vy,dz])),resolved=list(map(bits,[0,resolved,0])),
                    gravity=list(map(bits,gravity)),suppress=list(suppress),tag=tag,drag=single(drag),fail=fail,mutate=mutate,fall=bits(3.25))
    for flags in range(64):
        for entity in [0,-0.0,0.5,1,-1,float('nan'),float('inf')]:
            for block in [0,0.1,0.8,1,float('nan')]:
                for suppress in [(False,False),(True,),(False,True)]:
                    for tag in [False,True]:yield row(flags,entity,block,vy=[-2,-0.08,0][flags%3],suppress=suppress,tag=tag)
    for vy in [-float('inf'),-2,-math.nextafter(0.08,1),-0.08,-math.nextafter(0.08,0),-0.0,0,0.08,2,float('nan'),float('inf')]:
        for gravity in [(0,0),(.08,.08),(.08,.3),(float('nan'),.08)]:
            for flags in [4,12,15,31]:
                for entity in [0,.5]:
                    for resolved in [0,-0.1,vy]:yield row(flags,entity,vy=vy,gravity=gravity,resolved=resolved)
    for flags in [0,1,3,4,12,15,31,63]:
        for fail in range(-1,17):
            for mutate in [False,True]:yield row(flags,entity=.5,fail=fail,mutate=mutate)
    rng=random.Random(0x5245535449545554)
    for index in range(5000):
        values=[-2,-.08,-0.0,0,.08,.5,1,2,float('nan'),float('inf'),-float('inf')]
        yield row(rng.randrange(64),rng.choice(values),rng.choice(values),rng.choice(values),gravity=(rng.choice(values),rng.choice(values)),
                  suppress=(bool(rng.randrange(2)),bool(rng.randrange(2))),tag=bool(index%3==0),drag=rng.choice(values),
                  fail=rng.choice([-1,-1,-1,0,2,5,8,12]),mutate=bool(index%2),dx=rng.choice(values),dz=rng.choice(values),resolved=rng.choice(values))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-restitution-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaRestitutionOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/restitution_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaRestitutionOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'restitution_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} restitution output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.restituteMovementAfterCollisions over living/nonliving snapshots and controlled block restitution/tag behavior. Compares horizontal/vertical bounce, effective-gravity threshold, nonliving float scaling, air-drag interpolation, signed zero/nonfinite values and ordered lazy queries. Bounce callbacks can change velocity and fail; final velocity and syncPosition are checked after every injected failure boundary. This does not establish registered block restitution/tag provisioning, actual bounce sound/game-event effects, Entity.move guard conditions, fall damage, grounding or full gameplay.'}
    lab.write_json(report_path,report);print(f'Scoped restitution parity passed: {report_path}')


if __name__=='__main__':main()
