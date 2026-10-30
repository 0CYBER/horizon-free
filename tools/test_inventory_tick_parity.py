#!/usr/bin/env python3
"""Compare native InventoryTick transfers, shared stack identity and change callbacks."""
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
    edges=[-2147483648,-1,0,1,2,5,2147483647]
    def row(mode='both',**kw):
        v=dict(mode=mode,stacks=[dict(item=1,count=2,pop=5),dict(item=2,count=1,pop=2),dict(item=1,count=1,pop=0),dict(item=-1,count=0,pop=0)],slots=[[0,0],[1,0],[8,1],[35,2]],equipment=[[0,0],[1,1],[7,2]],selected=0,level=1,argument=0,argument_level=1,slot=-1,fail_at=0,mutate_at=0,mutate_ref=0,after_count=1,pop_ref=1,after_pop=5,replace_slot=8,replace_ref=2,equipment_slot=7,equipment_ref=0,after_selected=8,after_level=1);v.update(kw);return v
    for mode in ['inventory','equipment','both','stack']:
        for item in [-1,0,1,2]:
            for count in edges:
                for pop in edges:
                    for level in [0,1]:
                        v=row(mode,level=level,argument_level=level)
                        v['stacks'][0]=dict(item=item,count=count,pop=pop)
                        yield v
    for mode in ['inventory','equipment','both','stack']:
        for fail in range(1,9):
            for mutate in range(1,9):
                for count in [-1,0,2]:yield row(mode,fail_at=fail,mutate_at=mutate,after_count=count)
    # Inserts into an unpopulated EnumMap entry after iteration has begun.
    for initial in range(8):
        for inserted in range(8):
            for mutate in [1,2]:yield row('equipment',equipment=[[initial,0]],equipment_slot=inserted,mutate_at=mutate)
    rng=random.Random(0x494e565449434b)
    for _ in range(2000):
        v=row(rng.choice(['inventory','equipment','both','stack']),stacks=[dict(item=rng.randrange(-1,3),count=rng.choice(edges),pop=rng.choice(edges)) for __ in range(4)],slots=[[i,rng.randrange(4)] for i in rng.sample(range(43),rng.randrange(44))],equipment=[[i,rng.randrange(4)] for i in rng.sample(range(8),rng.randrange(9))],selected=rng.randrange(9),level=rng.randrange(2),argument=rng.randrange(4),argument_level=rng.randrange(2),slot=rng.randrange(-1,8),fail_at=rng.randrange(10),mutate_at=rng.randrange(10),mutate_ref=rng.randrange(4),after_count=rng.choice(edges),pop_ref=rng.randrange(4),after_pop=rng.choice(edges),replace_slot=rng.randrange(-1,43),replace_ref=rng.randrange(4),equipment_slot=rng.randrange(-1,8),equipment_ref=rng.randrange(4),after_selected=rng.randrange(9),after_level=rng.randrange(2))
        yield v


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-inventory-tick-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaInventoryTickOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/inventory_tick_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaInventoryTickOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'inventory_tick_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} inventory tick output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Inventory.tick, EntityEquipment.tick and ItemStack.inventoryTick composed over shared stacks. Custom item handlers trace server dispatch; level/item callbacks mutate counts, pop time, selection and future slot/enum-map entries or fail. Client JAR uses an actual allocated ClientLevel for the non-server branch; dedicated server uses null because ClientLevel is absent. Concrete registered item tick effects, arbitrary getItem overrides, whole entity/world ticks and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped inventory tick parity passed: {report_path}')


if __name__=='__main__':main()
