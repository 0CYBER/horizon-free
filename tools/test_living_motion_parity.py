#!/usr/bin/env python3
"""Compare default living jump, gravity and flight parameters with vanilla."""
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
    def row(**kw):
        f=dict(mode='jump',velocity=[.1,-.08,.2],delta=[0,.2,0],after_velocity=[17,18,19],strength=.42,block_factor=1,scale=1,boost=False,amplifier=0,sprint=False,yaw=37,sync=False,no_gravity=False,gravity=.08,slow_falling=False,passenger=0,speed=.1,omni=False,drag_modifier=1,fail=-1,mutate_at=-1)
        f.update(kw)
        for k in ['velocity','delta','after_velocity']:f[k]=[bits(v) for v in f[k]]
        for k in ['strength','gravity','drag_modifier']:f[k]=bits(f[k])
        for k in ['block_factor','scale','yaw','speed']:f[k]=single(f[k])
        return f
    vals=[-float('inf'),-1e30,-1,-.01,-1e-40,-0.0,0,1e-40,1e-5,.01,.1,.42,1,2,1e30,float('inf'),float('nan')]
    for strength in vals+[.000009999999,.000010000001]:
        for factor in vals:
            for scale in [-1,0,.5,1,float('nan')]:
                for boost in [False,True]:yield row(mode='power',strength=strength,block_factor=factor,scale=scale,boost=boost,amplifier=2)
    for amplifier in [-2147483648,-1,0,1,255,16777215,16777216,2147483647]:
        for boost in [False,True]:yield row(mode='boost',amplifier=amplifier,boost=boost)
    for strength in vals+[.000009999999,.000010000001]:
        for vy in vals:
            for sprint in [False,True]:
                for sync in [False,True]:yield row(strength=strength,velocity=[-0.0,vy,-0.0],sprint=sprint,sync=sync)
    for yaw in [*vals,*[i*360/1024 for i in range(1024)]]:yield row(sprint=True,yaw=yaw)
    for mode in ['gravity','effective','apply']:
        for gravity in vals:
            for vy in vals:
                for no_gravity in [False,True]:
                    for slow_falling in [False,True]:yield row(mode=mode,gravity=gravity,velocity=[-0.0,vy,-0.0],no_gravity=no_gravity,slow_falling=slow_falling)
    for speed in vals:
        for passenger in [0,1,2]:yield row(mode='speed',speed=speed,passenger=passenger)
    for drag_modifier in vals:
        for omni in [False,True]:yield row(mode='drag',drag_modifier=drag_modifier,omni=omni)
    vectors=[[0,0,0],[-0.0,-0.0,-0.0],[.1,.2,.3],[1,float('nan'),2],[float('inf'),0,0],[-float('inf'),1,0],[1.7976931348623157e308,1.7976931348623157e308,1.7976931348623157e308]]
    for v in vectors:
        for delta in vectors:
            for mode in ['add','up','down']:yield row(mode=mode,velocity=v,delta=delta)
    modes=['boost','power','jump','gravity','effective','apply','up','down','speed','drag','add']
    for mode in modes:
        for fail in range(-1,10):
            for mutate_at in range(-1,10):yield row(mode=mode,fail=fail,mutate_at=mutate_at,boost=True,sprint=True,slow_falling=True,passenger=2)
    rng=random.Random(0x4a554d50)
    for _ in range(8000):
        f={k:bool(rng.randrange(2)) for k in ['boost','sprint','sync','no_gravity','slow_falling','omni']}
        for k in ['strength','gravity','drag_modifier','block_factor','scale','yaw','speed']:f[k]=rng.choice(vals) if rng.randrange(3)==0 else rng.uniform(-1,2)
        yield row(**f,mode=rng.choice(modes),velocity=rng.choice(vectors),delta=rng.choice(vectors),after_velocity=rng.choice(vectors),amplifier=rng.choice([-2147483648,-1,0,1,255,2147483647]),passenger=rng.randrange(3),fail=rng.choice([-1,-1,*range(10)]),mutate_at=rng.randrange(-1,10))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-motion-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingMotionOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_motion_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingMotionOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_motion_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living-motion output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual default living jump power/boost/from-ground, gravity/effective-gravity, vertical liquid inputs, rider flight speed, air drag, Entity.applyGravity and finite addDeltaMovement. Traces field writes and lazy attribute/effect callbacks, both passenger classes, signed zero/nonfinite arithmetic, integer amplifier edges, callback mutations and failures. Effect amplifier fields are injected directly at boundaries. Entity-type jump/flight overrides, full effect/attribute storage, registered jump-factor binding and live gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living motion parity passed: {report_path}')


if __name__=='__main__':main()
