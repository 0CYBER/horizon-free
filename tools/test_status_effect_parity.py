#!/usr/bin/env python3
"""Compare shared status-effect storage with native instances."""
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
    durations=[-2147483648,-2,-1,0,1,2,20,32147,2147483647]
    amplifiers=[-2147483648,-1,0,1,2,255,256,2147483647]
    floats=[0,0x80000000,1,0x3f000000,0x3f800000,0xbf800000,0x40000000,0x7f800000,0xff800000,0x7fc00000,0x7f7fffff]
    def instance(**kw):
        v=dict(effect=0,duration=20,amplifier=1,ambient=False,visible=True,icon=True,hidden=-1);v.update(kw);return v
    def op(name,a=0,b=1,**kw):
        v=dict(op=name,a=a,b=b,value=2,factor=0x3f000000,previous=0,duration=10,amplifier=1,ambient=False,visible=True,icon=True,mutate=False,fail=False);v.update(kw);return v
    for a in durations:
        for b in durations:
            for amp in amplifiers:
                yield dict(instances=[instance(duration=a),instance(duration=b,amplifier=amp),instance(hidden=0)],ops=[op('update'),op('copy',a=0,b=1),op('eq'),op('same')])
    for duration in durations:
        for factor in floats:
            yield dict(instances=[instance(duration=duration),instance(hidden=0)],ops=[op('scale',factor=factor),op('eq'),op('ends',value=duration),op('infinite')])
    for flags in range(32):
        values=[instance(ambient=bool(flags&1),visible=bool(flags&2),icon=bool(flags&4)),instance(ambient=bool(flags&8),visible=bool(flags&16),icon=False),instance()]
        yield dict(instances=values,ops=[op('update'),op('copy',a=0,b=2),op('alias',a=2,b=1),op('update',a=1,b=2),op('set',amplifier=-99,factor=0x7fc00000),op('copy'),op('eq'),op('copy-blend',a=1,b=0),op('eq')])
    rng=random.Random(0x535441545553)
    for _ in range(1600):
        values=[instance(effect=rng.randrange(3),duration=rng.choice(durations),amplifier=rng.choice(amplifiers),ambient=bool(rng.randrange(2)),visible=bool(rng.randrange(2)),icon=bool(rng.randrange(2)),hidden=rng.randrange(-1,i)) for i in range(6)]
        ops=[op(rng.choice(['update','update','copy','alias','scale','ends','infinite','eq','same','set','map','copy-blend']),a=rng.randrange(6),b=rng.randrange(6),value=rng.choice(durations),factor=rng.choice(floats),previous=rng.choice(floats),duration=rng.choice(durations),amplifier=rng.choice(amplifiers),ambient=bool(rng.randrange(2)),visible=bool(rng.randrange(2)),icon=bool(rng.randrange(2)),mutate=bool(rng.randrange(2)),fail=bool(rng.randrange(2))) for _ in range(28)]
        yield dict(instances=values,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-status-effect-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStatusEffectOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/status_effect_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaStatusEffectOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'status_effect_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} status_effect output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual MobEffectInstance constructors, copy, update, duration mapping/scaling, equality and blend copying. Compares shared hidden-chain topology, aliases, raw and constructor-clamped amplifiers, infinite and overflow durations, copied scalar fields, reset blend state, and duration mapper mutations/failures. Mismatched-holder diagnostic logs are outside the comparison. Server/client ticking, effect behavior, registries and entity integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped status_effect parity passed: {report_path}')


if __name__=='__main__':main()
