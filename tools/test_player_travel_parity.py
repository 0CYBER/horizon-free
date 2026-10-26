#!/usr/bin/env python3
"""Compare Player travel wrapper and flying speed with vanilla."""
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
        f=dict(mode='travel',input=[.2,0,.8],velocity=[.1,-.08,.2],position=[.3,64,-.3],look=[0,.5,1],after_velocity=[17,18,19],after_position=[17,-64,-33],after_flying=False,base_velocity=[31,32,33],base_flying=False,base_mutate=True,passenger=False,swimming=True,jumping=False,empty=False,flying=True,sprint=False,speed=.05,fail=-1,mutate_at=-1)
        f.update(kw)
        for k in ['input','velocity','position','look','after_velocity','after_position','base_velocity']:f[k]=[bits(v) for v in f[k]]
        f['speed']=single(f['speed']);return f
    vals=[-float('inf'),-1e30,-1,-.20000000001,-.2,-.19999999999,-1e-40,-0.0,0,1e-40,.1,.5,1,1e30,float('inf'),float('nan')]
    for y in vals:
        for mask in range(128):
            flags={k:bool(mask&(1<<i)) for i,k in enumerate(['passenger','swimming','jumping','empty','flying','base_mutate','base_flying'])}
            yield row(look=[0,y,1],**flags)
    positions=[[.3,64,-.3],[-0.0,-0.0,-0.0],[17,-64,-33],[30000000,319,-30000000],[1,float('nan'),2],[float('inf'),1,-float('inf')],[2147483648,-2147483649,0]]
    for p in positions:
        for y in vals:
            for vy in vals:yield row(position=p,look=[0,y,0],velocity=[-0.0,vy,-0.0])
    for mode in ['speed','affected']:
        for speed in vals:
            for passenger in [False,True]:
                for flying in [False,True]:
                    for sprint in [False,True]:yield row(mode=mode,speed=speed,passenger=passenger,flying=flying,sprint=sprint)
    for mode in ['travel','speed','affected']:
        for fail in range(-1,14):
            for mutate_at in range(-1,14):
                for after_flying in [False,True]:yield row(mode=mode,fail=fail,mutate_at=mutate_at,after_flying=after_flying)
    rng=random.Random(0x504c41594552)
    for _ in range(10000):
        flags={k:bool(rng.randrange(2)) for k in ['passenger','swimming','jumping','empty','flying','base_mutate','base_flying','after_flying','sprint']}
        yield row(**flags,mode=rng.choice(['travel','travel','speed','affected']),position=rng.choice(positions),look=[0,rng.choice(vals),0],velocity=[rng.choice(vals),rng.choice(vals),rng.choice(vals)],after_velocity=[rng.choice(vals),rng.choice(vals),rng.choice(vals)],base_velocity=[rng.choice(vals),rng.choice(vals),rng.choice(vals)],speed=rng.choice(vals),fail=rng.choice([-1,-1,*range(14)]),mutate_at=rng.randrange(-1,14))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-player-travel-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerTravelOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/player_travel_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaPlayerTravelOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'player_travel_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} player-travel output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Player.travel, getFlyingSpeed and isAffectedByFluids. Covers passenger bypass, swim look pitch and head-fluid queries, flying vertical preservation, sprint speed, lazy ability access, provider mutations/failures, signed zero and nonfinite coordinates/velocity. Parent LivingEntity.travel selects an instrumented fluid branch; actual air/fluid collision simulation and full player/world gameplay remain separately pending integration.'}
    lab.write_json(report_path,report);print(f'Scoped player travel parity passed: {report_path}')


if __name__=='__main__':main()
