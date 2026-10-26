#!/usr/bin/env python3
"""Compare concrete consume effects and native status instance copies."""
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
    floats=[0,0x80000000,1,0x3f000000,0x3f800000,0xbf800000,0x40000000,0x7f800000,0xff800000,0x7fc00000]
    def instance(**kw):
        v=dict(duration=20,amplifier=1,ambient=False,visible=True,icon=True,factor=0x3f800000,previous=0x3f000000);v.update(kw);return v
    def config(**kw):
        v=dict(instances=[instance(),instance(duration=-1),instance(amplifier=2)],position=[1,64,-3],category=6,random=0,seeds=[123,456],reseed=False,returns=[False,True,False]);v.update(kw);return v
    def op(kind,**kw):
        v=dict(kind=kind,list=[0,1,0,2],probability=0x3f800000,level=0,sound=0);v.update(kw);return v
    def row(**kw):
        v=dict(initial=config(),after=config(instances=[instance(duration=7,amplifier=999),instance(duration=2147483647,amplifier=-999),instance(duration=0,amplifier=0)],position=[-100,300,200],category=7,random=1,reseed=True,returns=[True,False,True]),seeds=[0,1],xor=False,fail=-1,mutate=-1,ops=[op(0)]);v.update(kw);return v
    for xor in [False,True]:
        for kind in range(4):
            for fail in range(-1,7):
                for mutate in range(-1,7):yield row(xor=xor,ops=[op(kind)],fail=fail,mutate=mutate)
    for probability in floats:
        for indices in [[],[0],[1,1],[2,1,0,2]]:
            for seed in [0,1,-1,-9223372036854775808,9223372036854775807]:yield row(ops=[op(0,probability=probability,list=indices)],seeds=[seed,~seed])
    rng=random.Random(0x434f4e53454646)
    def random_instance():return instance(duration=rng.choice([-2147483648,-2,-1,0,1,20,2147483647]),amplifier=rng.choice([-2147483648,-1,0,1,255,256,2147483647]),ambient=bool(rng.randrange(2)),visible=bool(rng.randrange(2)),icon=bool(rng.randrange(2)),factor=rng.choice(floats),previous=rng.choice(floats))
    def random_config():return config(instances=[random_instance() for _ in range(3)],position=[rng.choice([-2147483648,-1,0,1,2147483647]) for _ in range(3)],category=rng.randrange(10),random=rng.randrange(2),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],reseed=bool(rng.randrange(2)),returns=[bool(rng.randrange(2)) for _ in range(5)])
    for _ in range(1800):yield row(initial=random_config(),after=random_config(),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],xor=bool(rng.randrange(2)),fail=rng.randrange(-1,30),mutate=rng.randrange(-1,30),ops=[op(rng.randrange(4),list=[rng.randrange(3) for _ in range(rng.randrange(6))],probability=rng.choice(floats),level=rng.randrange(2),sound=rng.randrange(2)) for _ in range(5)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-consume-effect-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaConsumeEffectOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/consume_effect_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaConsumeEffectOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'consume_effect_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} consume_effect output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual ApplyStatusEffects, RemoveStatusEffects, ClearAllStatusEffects and PlaySound consume effects. Uses real native effect instances and RNGs; compares probability edges, unconditional probability draws, fresh copy identity/details/hidden/blend state, ordered non-short-circuit add/remove calls, repeated holders, captured block positions, sound arguments and mutation/failure prefixes. Actor add/remove/clear storage and sound delivery remain controlled; named holder-set binding, teleport consumption and full gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped consume_effect parity passed: {report_path}')


if __name__=='__main__':main()
