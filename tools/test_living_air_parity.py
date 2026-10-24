#!/usr/bin/env python3
"""Compare LivingEntity air travel, climbing and input movement with vanilla."""
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
        f=dict(mode='air',input=[.4,0,.8],velocity=[.1,-.08,-.2],after_velocity=[.3,-.6,.9],fall=6,friction=.6,friction_modifier=1,drag_modifier=1,speed=.1,flying_speed=.02,yaw=37,ground=[True,True],climb=[False,False],scaffolding=False,suppress=False,player=False,horizontal=False,jumping=False,snow=False,after_horizontal=False,after_jumping=False,after_snow=False,powder_tag=False,boots=False,levitation=None,client=False,chunk=True,gravity=.08,y=64,min_y=-64,discard=False,omni=False,yaw_mutate=False,move_mutate=False,fail=-1)
        f.update(kw)
        for k in ['input','velocity','after_velocity']:f[k]=[bits(v) for v in f[k]]
        for k in ['fall','friction_modifier','drag_modifier','gravity','y']:f[k]=bits(f[k])
        for k in ['friction','speed','flying_speed','yaw']:f[k]=single(f[k])
        return f
    vals=[-float('inf'),-1e30,-1,-.15,-1e-40,-0.0,0,1e-40,.02,.1,.15,.6,.91,.98,1,2,1e30,float('inf'),float('nan')]
    for friction in vals:
        for modifier in vals:yield row(mode='friction',friction=friction,friction_modifier=modifier)
    for friction in vals+[.59999996,.60000002]:
        for speed in vals:
            for ground in [False,True]:yield row(mode='speed',friction=friction,speed=speed,flying_speed=speed,ground=[ground])
    vectors=[[0,0,0],[-0.0,-0.0,-0.0],[.0001,0,0],[math.sqrt(1e-7),0,0],[1,0,0],[0,0,1],[1,1,1],[float('inf'),0,0],[float('nan'),.1,0],[0,-.15000000596046448,0],[0,-float('inf'),0]]
    for i in range(4096):yield row(mode='input',yaw=i*360/4096,input=[.7,.3,-.8])
    for v in vectors:
        for yaw in vals+[90,180,270,360,-360]:
            for speed in [.1,-0.0,float('inf'),float('nan')]:yield row(mode='input',input=v,yaw=yaw,speed=speed)
    for v in vectors+[[.5,-.7,-.5],[0,-0.0,0]]:
        for climb in [False,True]:
            for player in [False,True]:
                for scaffolding in [False,True]:
                    for suppress in [False,True]:
                        for fail in range(-1,5):yield row(mode='climb',input=v,climb=[climb],player=player,scaffolding=scaffolding,suppress=suppress,fail=fail)
    for ground in [[False],[True],[True,False],[False,True]]:
        for climb in [[False],[True],[False,True],[True,False]]:
            for bits_mask in range(256):
                flags={k:bool(bits_mask&(1<<i)) for i,k in enumerate(['player','scaffolding','suppress','after_horizontal','after_jumping','after_snow','powder_tag','boots'])}
                yield row(mode='relative',ground=ground,climb=climb,**flags)
    for levitation in [None,0,1,255,2147483647]:
        for client in [False,True]:
            for chunk in [False,True]:
                for y in [-float('inf'),-64,-63,0,float('inf'),float('nan')]:
                    for discard in [False,True]:
                        for omni in [False,True]:
                            for fail in range(-1,24):yield row(levitation=levitation,client=client,chunk=chunk,y=y,discard=discard,omni=omni,fail=fail,move_mutate=True,yaw_mutate=True,climb=[True],after_horizontal=True,player=True,suppress=True)
    rng=random.Random(0x414952)
    flags=['player','scaffolding','suppress','horizontal','jumping','snow','after_horizontal','after_jumping','after_snow','powder_tag','boots','client','chunk','discard','omni','yaw_mutate','move_mutate']
    for _ in range(12000):
        f={k:bool(rng.randrange(2)) for k in flags}
        for k in ['friction','speed','flying_speed','friction_modifier','drag_modifier','gravity','yaw']:f[k]=rng.choice(vals) if rng.randrange(3)==0 else rng.uniform(-2,2)
        yield row(**f,input=rng.choice(vectors),velocity=rng.choice(vectors),after_velocity=rng.choice(vectors),ground=[bool(rng.randrange(2)),bool(rng.randrange(2))],climb=[bool(rng.randrange(2)),bool(rng.randrange(2))],levitation=rng.choice([None,None,0,1,255,2147483647]),fail=rng.choice([-1,-1,*range(24)]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-air-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingAirOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_air_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingAirOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_air_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living-air output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity.travelInAir, handleRelativeFrictionAndCalculateMovement, handleOnClimbable, getFrictionInfluencedSpeed, computeModifiedFriction and Entity.getInputVector. Traces native moveRelative and state writes, both Player/nonplayer ladder guards, registered scaffolding/boots checks, levitation, missing client chunks, float friction/drag, callback mutations and failures. Entity.move, effective gravity, ground/climb predicates and attributes use controlled providers; live world integration and complete player travel remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living air parity passed: {report_path}')


if __name__=='__main__':main()
