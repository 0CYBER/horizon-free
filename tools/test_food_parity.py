#!/usr/bin/env python3
"""Compare food arithmetic, exhaustion and server hunger tick with vanilla."""
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
    def bits(v):return struct.pack('>f',v).hex()
    def state(food=20,saturation=5,exhaustion=0,timer=0):return dict(food=food,saturation=bits(saturation),exhaustion=bits(exhaustion),timer=timer)
    def world(difficulty=2,regen=True,hurt=True,health=10):return dict(difficulty=difficulty,regen=regen,hurt=hurt,health=bits(health))
    def op(mode='tick',nutrition=0,amount=0):return dict(mode=mode,nutrition=nutrition,amount=bits(amount))
    def row(initial,ops,**kw):
        f=dict(initial=initial,ops=ops,world=world(),after=state(0,0,7,79),after_world=world(3,False,False,1),fail=-1,mutate_at=-1);f.update(kw);return f
    edges=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,4,5,6,20,40,1e30,float('inf'),float('nan')]
    foods=[-2147483648,-1,0,1,6,17,18,19,20,21,2147483647]
    timers=[-2147483648,-1,0,8,9,10,78,79,80,2147483647]
    for food in foods:
        for nutrition in foods:
            for value in edges:
                for saturation in [-1.,-0.,0.,5.,float('nan')]:
                    yield row(state(food,saturation),[op('eat',nutrition,value),op('properties',nutrition,value),op('exhaust',amount=value)])
    for difficulty in range(4):
        for regen in [False,True]:
            for hurt in [False,True]:
                for food in [-1,0,17,18,19,20,21]:
                    for saturation in [0.,.1,5.,6.,float('nan')]:
                        for timer in [0,9,79]:
                            yield row(state(food,saturation,4.000001,timer),[op(),op()],world=world(difficulty,regen,hurt,1))
    for health in edges:
        for difficulty in range(4):yield row(state(0,0,0,79),[op()],world=world(difficulty,False,True,health))
    for initial in [state(20,5,0,9),state(18,0,0,79),state(0,0,0,79),state(20,5,5,9)]:
        for fail in range(-1,13):
            for mutation in range(-1,13):yield row(initial,[op()],fail=fail,mutate_at=mutation)
    rng=random.Random(0x464f4f44)
    for _ in range(3000):
        ops=[op(rng.choice(['tick','tick','tick','eat','properties','exhaust','food','saturation']),rng.choice(foods),rng.choice(edges)) for __ in range(rng.randrange(1,50))]
        yield row(state(rng.choice(foods),rng.choice(edges),rng.choice(edges),rng.choice(timers)),ops,world=world(rng.randrange(4),bool(rng.randrange(2)),bool(rng.randrange(2)),rng.choice(edges)),fail=rng.choice([-1,-1,0,3,5,9]),mutate_at=rng.choice([-1,-1,1,4,6,8]))
    for initial in [state(20,5),state(18,0),state(0,0)]:yield row(initial,[op() for _ in range(240)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-food-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFoodOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/food_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFoodOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'food_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} food output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual FoodData eating/properties, exhaustion, hunger predicates and server tick, including query order, mutations/failures, regeneration thresholds, starvation difficulty/health guards, signed-zero/nonfinite values and integer overflow. Player healing/damage and world queries are controlled; full damage/health, consumables, persistence and playable integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped food parity passed: {report_path}')


if __name__=='__main__':main()
