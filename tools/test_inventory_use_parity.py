#!/usr/bin/env python3
"""Compare native living item-use tick, release, completion and stop lifecycle."""
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
    modes=['tick','update','complete','release','stop']
    ints=[-2147483648,-1,0,1,2,20,2147483647]
    def state(**kw):
        s=dict(item=0,remaining=1,virtual_remaining=1,kinetic=True,selected=0,counts=[1,1,0],slots=[[0,0],[8,1],[40,1]],level=0,clients=[False,True],releases=[False,True],finishes=[1,0]);s.update(kw);return s
    def row(mode='tick',**kw):
        f=dict(mode=mode,argument=0,flags=1,shared_type=False,vibrations=[True,True],initial=state(),after=state(item=1,remaining=0,virtual_remaining=-1,counts=[0,2,0],selected=8,slots=[[0,1],[8,0],[40,0]],level=1),fail=-1,mutate_at=-1);f.update(kw);return f
    for mode in modes:
        for flags in range(256):
            for selected in [0,1,8]:yield row(mode,flags=flags,initial=state(selected=selected))
        for item in range(3):
            for count in ints:
                for remaining in ints:
                    yield row(mode,argument=item,initial=state(item=item,counts=[count,count,count],remaining=remaining,virtual_remaining=remaining))
        for mutate in range(-1,12):
            for fail in range(-1,12):yield row(mode,mutate_at=mutate,fail=fail)
    rng=random.Random(0x494e56555345)
    def random_state():return state(item=rng.randrange(3),remaining=rng.choice(ints),virtual_remaining=rng.choice(ints),kinetic=bool(rng.randrange(2)),selected=rng.randrange(9),counts=[rng.choice(ints) for _ in range(3)],slots=[[i,rng.randrange(3)] for i in rng.sample(range(43),rng.randrange(12))],level=rng.randrange(2),clients=[bool(rng.randrange(2)),bool(rng.randrange(2))],releases=[bool(rng.randrange(2)),bool(rng.randrange(2))],finishes=[rng.randrange(3),rng.randrange(3)])
    for _ in range(5500):yield row(rng.choice(modes),argument=rng.randrange(3),flags=rng.randrange(256),shared_type=bool(rng.randrange(2)),vibrations=[bool(rng.randrange(2)),bool(rng.randrange(2))],initial=random_state(),after=random_state(),fail=rng.choice([-1,-1,*range(12)]),mutate_at=rng.randrange(-1,12))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-inventory-use-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaInventoryUseOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/inventory_use_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaInventoryUseOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'inventory_use_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} inventory use output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual inherited LivingEntity use tick/update/completion/release/stop composed with real Player Inventory, PlayerEquipment hand access and byte SynchedEntityData flags. Traces compare held/active reference identity, live count mutations, selected/offhand writes, onEquipItem callbacks, synchronized value/dirty prefixes, finish vibration and kinetic-map presence. Custom item/level effects remain controlled; starting use, complete onEquipItem effects, consumables, kinetic combat, remainders/cooldowns and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped inventory use parity passed: {report_path}')


if __name__=='__main__':main()
