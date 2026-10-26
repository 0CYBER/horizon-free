#!/usr/bin/env python3
"""Compare water-entry sound, particle and random draw ordering with vanilla."""
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
        f=dict(position=[17.25,64.9,-33.5],width=.6,velocity=[.2,-.3,.1],other_velocity=[.8,-.9,1.2],after_position=[31.5,100.9,-52.25],after_width=.1,after_velocity=[1,2,3],after_other_velocity=[4,5,6],passenger=0,legacy=True,seed=42,fail=-1,mutate_at=-1);f.update(kw)
        for k in ['position','velocity','other_velocity','after_position','after_velocity','after_other_velocity']:f[k]=[bits(v) for v in f[k]]
        for k in ['width','after_width']:f[k]=single(f[k])
        return f
    yield row()
    widths=[-float('inf'),-1,-.050000004,-.05,-.049999997,-0.,0.,.0001,.049999997,.05,.050000004,.1,.6,1,2,float('nan')]
    vectors=[[0,0,0],[1,0,0],[0,1.2499999,0],[0,1.25,0],[0,1.2500001,0],[0,.2777777777777778,0],[10,20,30],[1,float('nan'),2],[float('inf'),0,0],[-0.,-0.,-0.]]
    for passenger in range(3):
        for legacy in [False,True]:
            for width in widths:
                for velocity in vectors:yield row(passenger=passenger,legacy=legacy,width=width,velocity=velocity,other_velocity=velocity)
    for legacy in [False,True]:
        for passenger in range(3):
            for fail in range(-1,45):
                for mutate in [-1,0,1,2,3,4,5,6,7,8,9,10,20,30]:yield row(legacy=legacy,passenger=passenger,fail=fail,mutate_at=mutate)
    for y in [-float('inf'),-1e30,-2147483649,-.1,-0.,0.,.9,16777215,16777216,2147483648,1e30,float('inf'),float('nan')]:yield row(position=[-0.,y,float('nan')],width=.01)
    for width in [100000,float('inf')]:yield row(width=width,fail=7)
    rng=random.Random(0x53504c415348)
    for _ in range(3000):yield row(seed=rng.randrange(-2**63,2**63),legacy=bool(rng.randrange(2)),passenger=rng.randrange(3),width=rng.choice(widths),velocity=rng.choice(vectors),other_velocity=rng.choice(vectors),after_width=rng.choice(widths),fail=rng.choice([-1,-1,*range(35)]),mutate_at=rng.choice([-1,-1,*range(35)]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-water-splash-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaWaterSplashOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/water_splash_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaWaterSplashOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'water_splash_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} water-splash output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.doWaterSplashEffect with native LegacyRandomSource and XoroshiroRandomSource. Compares controlling-passenger identity, captured velocity, sound selection/volume/pitch, ordered bubble/splash particle requests, float-rounded surface height, random draw continuation and final game event. Includes width/position/velocity mutations, nonfinite arithmetic and injected failures. Sound identities and effect sinks are controlled; audio playback, particle simulation/rendering and complete gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped water splash parity passed: {report_path}')


if __name__=='__main__':main()
