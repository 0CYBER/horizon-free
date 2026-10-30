#!/usr/bin/env python3
"""Compare LivingEntity water/lava travel and fluid velocity updates with vanilla."""
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
        f=dict(mode='fluid',input=[.4,0,.8],velocity=[.1,-.08,-.2],after_velocity=[.3,-.6,.9],fall=6,friction=.6,friction_modifier=1,drag_modifier=1,speed=.1,flying_speed=.02,yaw=37,ground=[True,True],climb=[False,False],scaffolding=False,suppress=False,player=False,horizontal=False,jumping=False,snow=False,after_horizontal=False,after_jumping=False,after_snow=False,powder_tag=False,boots=False,levitation=None,client=False,chunk=True,gravity=.08,y=64,min_y=-64,discard=False,omni=False,yaw_mutate=False,move_mutate=False,fail=-1)
        f.update(water=True,sprint=[False],slow_down=.8,efficiency=0,dolphin=False,lava_height=1,floatable_height=0,threshold=.4,free=False,vehicle=False,float_tag=False,free_mutate=False,falling=True,old_y=64,after_y=64)
        f.update(kw)
        for k in ['input','velocity','after_velocity']:f[k]=[bits(v) for v in f[k]]
        for k in ['fall','friction_modifier','drag_modifier','gravity','y','efficiency','lava_height','floatable_height','threshold','old_y','after_y']:f[k]=bits(f[k])
        for k in ['friction','speed','flying_speed','yaw','slow_down']:f[k]=single(f[k])
        return f
    vals=[-float('inf'),-1e30,-1,-.15,-.003,-1e-40,-0.0,0,1e-40,.002,.003,.005,.008,.08,.15,.6,.91,.98,1,2,1e30,float('inf'),float('nan')]
    for gravity in vals:
        for y in vals+[math.nextafter(.002,0),math.nextafter(.002,1),math.nextafter(.008,0),math.nextafter(.008,1)]:
            for falling in [False,True]:
                for sprint in [False,True]:yield row(mode='adjust',input=[-0.0,y,float('nan')],gravity=gravity,falling=falling,sprint=[sprint])
    for efficiency in vals:
        for slowdown in vals:
            for ground in [False,True]:
                for dolphin in [False,True]:
                    for sprint in [[False],[True],[False,True],[True,False]]:yield row(mode='water',efficiency=efficiency,slow_down=slowdown,ground=[ground],dolphin=dolphin,sprint=sprint)
    for height in vals:
        for threshold in vals:
            for gravity in [-0.0,.08,float('nan')]:
                for falling in [False,True]:yield row(mode='lava',lava_height=height,threshold=threshold,gravity=gravity,falling=falling,move_mutate=True)
    for mode in ['water','lava','fluid','jump','float']:
        for mask in range(256):
            f={k:bool(mask&(1<<i)) for i,k in enumerate(['water','after_horizontal','free','vehicle','float_tag','dolphin','move_mutate','free_mutate'])}
            yield row(mode=mode,**f,climb=[True],floatable_height=1,threshold=.4,horizontal=f['after_horizontal'])
    for mode in ['water','lava','fluid','jump','float']:
        for fail in range(-1,30):
            for water in [False,True]:
                for free in [False,True]:
                    for shallow in [False,True]:yield row(mode=mode,fail=fail,water=water,free=free,lava_height=.1 if shallow else 2,vehicle=True,float_tag=True,floatable_height=1,move_mutate=True,free_mutate=True,horizontal=True,after_horizontal=True,climb=[True])
    rng=random.Random(0x464c554944)
    flags=['water','horizontal','after_horizontal','dolphin','vehicle','float_tag','free','move_mutate','free_mutate','falling','player','yaw_mutate']
    vectors=[[.3,-.08,.7],[-0.0,-0.0,-0.0],[1,float('nan'),2],[float('inf'),-float('inf'),0],[.005,.003,.01]]
    for _ in range(10000):
        f={k:bool(rng.randrange(2)) for k in flags}
        for k in ['gravity','efficiency','lava_height','floatable_height','threshold','old_y','y','after_y']:f[k]=rng.choice(vals) if rng.randrange(3)==0 else rng.uniform(-2,2)
        yield row(**f,mode=rng.choice(['water','lava','fluid','jump','float','adjust']),input=rng.choice(vectors),velocity=rng.choice(vectors),after_velocity=rng.choice(vectors),slow_down=rng.choice(vals),sprint=[bool(rng.randrange(2)),bool(rng.randrange(2))],ground=[bool(rng.randrange(2))],climb=[bool(rng.randrange(2))],fail=rng.choice([-1,-1,*range(25)]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-fluid-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingFluidOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_fluid_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingFluidOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_fluid_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living-fluid output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity water/lava travel, fluid falling adjustment, shallow-fluid branch, fluid escape and ridden buoyancy. Includes native input acceleration and field write ordering, sprint changes, efficiency/dolphin effects, movement and free-space mutations, nonfinite arithmetic and injected failures. Controlled collision movement, immersion depths, free-space results, attributes and gravity; live fluid propagation, player/world binding and complete gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living fluid parity passed: {report_path}')


if __name__=='__main__':main()
