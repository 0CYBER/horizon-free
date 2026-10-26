#!/usr/bin/env python3
"""Compare player hunger guards, exhaustion and direct/virtual state binding with vanilla."""
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
    def state(food=20,other=0,invulnerable=False,mayfly=False,client=False,direct=0,virtual=1,exhaustion=0):return dict(foods=[dict(food=food,exhaustion=bits(exhaustion)),dict(food=other,exhaustion=bits(exhaustion))],invulnerable=invulnerable,mayfly=mayfly,client=client,direct=direct,virtual=virtual)
    def row(mode,initial,**kw):
        f=dict(mode=mode,initial=initial,after=state(1,20,True,True,False,1,0,7),amount=bits(.1),always=False,stored=False,fail=-1,mutate_at=-1);f.update(kw);return f
    foods=[-2**31,-1,0,1,5,6,7,19,20,21,2**31-1]
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,39.9,40,1e30,float('inf'),float('nan')]
    for food in foods:
        for flags in range(32):
            initial=state(food,6,flags&1!=0,flags&2!=0,flags&4!=0,(flags>>3)&1,(flags>>4)&1)
            for mode in ['exhaust','eat','enough']:
                for always in [False,True]:yield row(mode,initial,always=always)
    for flags in range(8):
        for mode in ['exhaust','eat','enough']:
            for fail in range(-1,5):
                for mutation in range(-1,5):yield row(mode,state(0,0,flags&1!=0,flags&2!=0,flags&4!=0),fail=fail,mutate_at=mutation)
    for v in values:
        for exhaustion in values:
            for stored in [False,True]:yield row('exhaust',state(20,0,virtual=0,exhaustion=exhaustion),amount=bits(v),stored=stored)
    rng=random.Random(0x504c4159464f4f44)
    for _ in range(2500):
        stored=bool(rng.randrange(2));initial=state(rng.choice(foods),rng.choice(foods),bool(rng.randrange(2)),bool(rng.randrange(2)),bool(rng.randrange(2)),0 if stored else rng.randrange(2),0 if stored else rng.randrange(2),rng.choice(values))
        yield row(rng.choice(['exhaust','eat','enough']),initial,amount=bits(rng.choice(values)),always=bool(rng.randrange(2)),stored=stored,fail=-1 if stored else rng.choice([-1,-1,0,1,2,3]),mutate_at=-1 if stored else rng.choice([-1,-1,0,1,2]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-player-food-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerFoodOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/player_food_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaPlayerFoodOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'player_food_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} player food output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Player causeFoodExhaustion/canEat/hasEnoughFoodToDoExhaustiveManoeuvres with native FoodData. Direct abilities/food fields versus virtual food/ability getters, captured food identity across callbacks, client/server and invulnerability guards, mutations/failures, nonfinite exhaustion and integer hunger boundaries. Also tests the ordinary stored-food binding. Consumable/item components, abilities persistence and live player tick remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped player food parity passed: {report_path}')


if __name__=='__main__':main()
