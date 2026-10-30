#!/usr/bin/env python3
"""Compare consumable start/completion with food listeners and particles."""
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
    floats=[0,0x80000000,1,0x3d000000,0x3d4ccccd,0x3f000000,0x3f800000,0x3fcccccd,0xbf800000,0x42b40000,0xc2b40000,0x7f7fffff,0x7f800000,0xff800000,0x7fc00000]
    def food(**kw):
        v=dict(nutrition=4,saturation=0x40a00000,always=False);v.update(kw);return v
    def config(**kw):
        v=dict(position=[1.25,64.5,-2.75],pitch=0,yaw=0,eye=0x3fcf5c29,count=3,damage=7,random=0,virtual_random=0,level=0,sound=0,reseed=False,seeds=[99,101],food_component=food(),food=10,saturation=0x40a00000,can_eat=True,client=False,infinite=False,effect_result=False);v.update(kw);return v
    def row(**kw):
        v=dict(initial=config(),after=config(position=[-3.5,71.25,9.5],pitch=0x42b40000,yaw=0xc2b40000,count=1,damage=99,random=1,virtual_random=1,level=1,sound=1,reseed=True,food_component=food(nutrition=20,always=True),food=4,saturation=0x80000000,can_eat=False,client=True,infinite=True,effect_result=True),seeds=[0,1],item=2,xor=False,fail=-1,mutate=-1,animation=1,seconds=0,remaining=24,sound=0,has_particles=False,emit_random=0,override=False,actor=2,effects=3,op=2,hand=0,passed_level=0);v.update(kw);return v
    for xor in [False,True]:
        for actor in range(3):
            for op in [1,2]:
                for fail in range(-1,15):
                    for mutate in range(-1,15):yield row(xor=xor,actor=actor,op=op,fail=fail,mutate=mutate)
    for actor in range(3):
        for item in range(3):
            for count in [-1,0,1,2,2147483647]:
                for component in [None,food(),food(always=True)]:
                    for can_eat in [False,True]:
                        for seconds in [0,0x3fcccccd]:yield row(actor=actor,item=item,initial=config(count=count,food_component=component,can_eat=can_eat),seconds=seconds,op=1)
    for seconds in floats:
        for op in range(3):yield row(seconds=seconds,op=op)
    for actor in range(3):
        for animation in [1,2]:
            for mutate in [-1,0,1,5,90,112,114,116,119,122,125]:yield row(actor=actor,animation=animation,mutate=mutate,has_particles=True)
    rng=random.Random(0x434f4e53554d4544)
    def random_config():return config(position=[rng.choice([-0.0,0.0,1.25,-15.5,29999999.5]) for _ in range(3)],pitch=rng.choice(floats),yaw=rng.choice(floats),eye=rng.choice(floats),count=rng.choice([-1,0,1,16,2147483647]),damage=rng.choice([None,-1,0,99,2147483647]),random=rng.randrange(2),virtual_random=rng.randrange(2),level=rng.randrange(2),reseed=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],food_component=rng.choice([None,food(nutrition=rng.choice([-2147483648,-1,0,1,20,2147483647]),saturation=rng.choice(floats),always=bool(rng.randrange(2)))]),food=rng.choice([-1,0,1,19,20,2147483647]),saturation=rng.choice(floats),can_eat=bool(rng.randrange(2)),client=bool(rng.randrange(2)),infinite=bool(rng.randrange(2)),effect_result=bool(rng.randrange(2)))
    for _ in range(2400):yield row(initial=random_config(),after=random_config(),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],item=rng.randrange(3),xor=bool(rng.randrange(2)),fail=rng.choice([-1,-1,rng.randrange(128)]),mutate=rng.choice([-1,-1,rng.randrange(128)]),animation=rng.randrange(12),seconds=rng.choice(floats),sound=rng.randrange(2),has_particles=bool(rng.randrange(2)),actor=rng.randrange(3),effects=rng.randrange(4),op=rng.randrange(3),hand=rng.randrange(2),passed_level=rng.randrange(2))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-consumable-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaConsumableOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/consumable_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaConsumableOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'consumable_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} consumable output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Consumable.canConsume/startConsuming/onConsume composed with inherited eating particles, real FoodProperties/FoodData, stack components and both native RNGs. Covers player/non-player/server-player branches, stat item selection, empty advancement trigger storage, food listener dispatch, client/server effect guards, event and finite/infinite stack consumption, captured component values and callback mutation/failure prefixes. Food eligibility/start-use, stats, effect bodies and event delivery are controlled services; only the ordinary food listener schema is bound here. Full advancement evaluation, potion effects, lifecycle integration and playable gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped consumable parity passed: {report_path}')


if __name__=='__main__':main()
