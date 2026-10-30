#!/usr/bin/env python3
"""Compare native food propertiess and strict scalar stack validation."""
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
    edges=[-2147483648,-1,0,1,19,20,21,2147483647]
    floats=[0,0x80000000,1,0x3f000000,0x3f800000,0x40a00000,0xbf800000,0x7f800000,0xff800000,0x7fc00000]
    seeds=[0,1,-1,-9223372036854775808,9223372036854775807,123456789]
    def food(food=10,saturation=0x40a00000,exhaustion=0,timer=0):return dict(food=food,saturation=saturation,exhaustion=exhaustion,timer=timer)
    def config(**kw):
        v=dict(position=[1.25,64.5,-2.75],foods=[food(),food(20)],random=0,food=0,reseed=False,seeds=[99,101]);v.update(kw);return v
    def row(**kw):
        v=dict(initial=config(),after=config(position=[-3.5,71.25,9.5],foods=[food(2,0x80000000),food(18)],random=1,food=1,reseed=True),seeds=[0,1],player=True,xor=False,nutrition=4,saturation=0x40a00000,always=False,fail=-1,mutate=-1);v.update(kw);return v
    for xor in [False,True]:
        for seed in seeds:
            for player in [False,True]:
                for fail in range(-1,4):
                    for mutate in range(-1,4):yield row(xor=xor,seeds=[seed,~seed],player=player,fail=fail,mutate=mutate)
    for nutrition in edges:
        for saturation in floats:
            for level in edges:yield row(nutrition=nutrition,saturation=saturation,initial=config(foods=[food(level,saturation),food()]))
    rng=random.Random(0x464f4f444c495354)
    def random_food():return food(rng.choice(edges),rng.choice(floats),rng.choice(floats),rng.choice(edges))
    def random_config():return config(position=[rng.choice([-0.0,0.0,1.25,-15.5,29999999.5]) for _ in range(3)],foods=[random_food(),random_food()],random=rng.randrange(2),food=rng.randrange(2),reseed=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)])
    for _ in range(2400):yield row(initial=random_config(),after=random_config(),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],player=bool(rng.randrange(2)),xor=bool(rng.randrange(2)),nutrition=rng.choice(edges),saturation=rng.choice(floats),always=bool(rng.randrange(2)),fail=rng.randrange(-1,4),mutate=rng.randrange(-1,4))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-food-properties-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFoodPropertiesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/food_properties_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFoodPropertiesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'food_properties_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} food properties output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual FoodProperties.onConsume with real stored FoodData, native LegacyRandomSource and XoroshiroRandomSource, player/non-player class selection, and controlled random/food getter and level sound callbacks. Compares sound arguments, stored hunger/saturation, captured random-source selection, subsequent RNG state, position reads and mutation/failure prefixes. Rust uses the existing FoodData and random stream implementations. Sound mixing/playback, concrete consumable dispatch, particles/effects and full gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped food properties parity passed: {report_path}')


if __name__=='__main__':main()
