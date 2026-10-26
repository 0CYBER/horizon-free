#!/usr/bin/env python3
"""Compare native status-effect ticking, hidden restoration and blending."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    durations=[-2147483648,-2,-1,0,1,2,3,20,2147483647]
    floats=[0,0x80000000,1,0x3f000000,0x3f800000,0xbf800000,0x40000000,0x7f800000,0xff800000,0x7fc00000]
    def instance(**kw):
        v=dict(effect=0,duration=20,amplifier=1,ambient=False,visible=True,icon=True,hidden=-1,factor=0x3f000000,previous=0);v.update(kw);return v
    def config(**kw):
        v=dict(instances=[instance(duration=8),instance(duration=1,hidden=0),instance(duration=-1,hidden=1)],tick=99,should=True,apply=True,advance=2,blend_in=4,blend_out=8);v.update(kw);return v
    def op(name,a=1,b=0,**kw):
        v=dict(op=name,a=a,b=b,level=0,partial=0x3f000000,removed=False);v.update(kw);return v
    def row(**kw):
        v=dict(initial=config(),after=config(instances=[instance(duration=-1,amplifier=999),instance(duration=1,amplifier=-9),instance(duration=0)],tick=2147483647,should=False,apply=False,advance=0,blend_in=0,blend_out=-2),fail=-1,mutate=-1,ops=[op('server'),op('client'),op('factor')]);v.update(kw);return v
    for mode in ['server','client','skip']:
        for fail in range(-1,5):
            for mutate in range(-1,5):yield row(fail=fail,mutate=mutate,ops=[op(mode),op('factor')])
    for duration in durations:
        for hidden_duration in durations:
            for should in [False,True]:
                for apply in [False,True]:yield row(initial=config(instances=[instance(duration=hidden_duration),instance(duration=duration,hidden=0),instance(duration=3,hidden=1)],should=should,apply=apply),ops=[op('server'),op('server'),op('client'),op('client'),op('factor')])
    for value in floats:
        for blend_ticks in [-2147483648,-2,-1,0,1,2,20,2147483647]:
            for duration in [-1,0,1,20]:yield row(initial=config(instances=[instance(factor=value),instance(duration=duration,factor=value),instance(factor=value)],blend_in=blend_ticks,blend_out=blend_ticks),ops=[op('client'),op('factor',partial=value),op('factor',removed=True),op('skip'),op('factor',partial=value)])
    rng=random.Random(0x4546465449434b)
    def random_config():return config(instances=[instance(effect=rng.randrange(3),duration=rng.choice(durations),amplifier=rng.choice([-2147483648,-1,0,1,255,256,2147483647]),ambient=bool(rng.randrange(2)),visible=bool(rng.randrange(2)),icon=bool(rng.randrange(2)),hidden=rng.randrange(-1,i),factor=rng.choice(floats),previous=rng.choice(floats)) for i in range(3)],tick=rng.choice(durations),should=bool(rng.randrange(2)),apply=bool(rng.randrange(2)),advance=rng.choice(durations),blend_in=rng.choice(durations),blend_out=rng.choice(durations))
    for _ in range(1800):yield row(initial=random_config(),after=random_config(),fail=rng.randrange(-1,20),mutate=rng.randrange(-1,20),ops=[op(rng.choice(['server','client','skip','factor','update','copy','alias']),a=rng.randrange(3),b=rng.randrange(3),level=rng.randrange(2),partial=rng.choice(floats),removed=bool(rng.randrange(2))) for _ in range(16)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-status-tick-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStatusTickOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/status_tick_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaStatusTickOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'status_tick_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} status_tick output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual MobEffectInstance server/client ticks, skipBlending and getBlendFactor with shared instance/hidden-chain storage. Controlled MobEffect scheduling/application/blend-duration callbacks compare duration countdown, infinite actor clocks, hidden restoration and notification, live-state rereads, float interpolation, removed actors and callback mutation/failure prefixes. Concrete registered effect behaviors, entity effect maps, UI rendering and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped status_tick parity passed: {report_path}')


if __name__=='__main__':main()
