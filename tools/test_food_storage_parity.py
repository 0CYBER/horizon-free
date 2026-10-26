#!/usr/bin/env python3
"""Compare vanilla food NBT persistence and numeric conversions with vanilla."""
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
    def double(v):return struct.pack('>d',v).hex()
    pool=[dict(type=i,value=0) for i in [0,7,8,9,10,11,12]]
    for t,values in [(1,[-128,-1,0,1,127]),(2,[-32768,-129,-1,0,1,32767]),(3,[-2**31,-1,0,1,20,2**31-1]),(4,[-2**63,-2**32-1,-2**31,-1,0,1,2**31,2**32+1,2**63-1])]:
        pool.extend(dict(type=t,value=str(v) if t==4 else v) for v in values)
    values=[-float('inf'),-1e30,-2147483649.,-1.9,-1.,-0.,0.,1e-40,.1,1.,1.9,20.,2147483647.,2147483648.,1e30,float('inf'),float('nan')]
    pool.extend(dict(type=t,value=(bits(v) if t==5 else double(v))) for t in [5,6] for v in values)
    pool.extend([dict(type=5,value='7fa12345'),dict(type=6,value='7ff123456789abcd')])
    for slot in range(4):
        for item in pool:
            tags=[dict(type=0,value=0) for _ in range(4)];tags[slot]=item;yield dict(mode='read',tags=tags)
    rng=random.Random(0x464f4f444e4254)
    for _ in range(6000):yield dict(mode='read',tags=[rng.choice(pool) for __ in range(4)])
    for a in values:
        for b in values:yield dict(mode='write',food=rng.choice([-2**31,-1,0,20,2**31-1]),timer=rng.choice([-2**31,-1,0,79,2**31-1]),saturation=bits(a),exhaustion=bits(b))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-food-storage-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFoodStorageOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/food_storage_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFoodStorageOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'food_storage_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} food storage output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual FoodData readAdditionalSaveData/addAdditionalSaveData through native TagValueInput/TagValueOutput; all NBT numeric types, missing/wrong-type defaults, numeric coercion, overflow, nonfinite values, signed-zero normalization, output types and round trips. Other player fields retained. ProblemReporter diagnostics, complete player save lifecycle and playable integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped food storage parity passed: {report_path}')


if __name__=='__main__':main()
