#!/usr/bin/env python3
"""Compare native startUsingItem equipment, timer and flag ordering."""
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
    ints=[-2147483648,-1,0,1,2,20,72000,2147483647]
    def state(**kw):
        s=dict(item=2,remaining=3,virtual_remaining=1,kinetic=True,using=False,off=False,hand=0,hands=[0,1],level=0,clients=[False,True],releases=[False,True],finishes=[1,0],durations=[72000,20]);s.update(kw);return s
    def row(**kw):
        f=dict(start_hand=0,shared_type=False,vibrations=[True,True],kinetic_items=[True,False],initial=state(),after=state(item=1,remaining=0,off=True,using=False,hand=1,hands=[1,0],level=1,clients=[False,False],durations=[5,9]),fail=-1,mutate_at=-1);f.update(kw);return f
    for item in range(3):
        for held in range(3):
            for hand in range(2):
                for duration in ints:
                    for using in [False,True]:
                        for client in [False,True]:
                            for shared in [False,True]:
                                yield row(start_hand=hand,shared_type=shared,initial=state(item=item,hands=[held,held],durations=[duration,duration],using=using,clients=[client,client]))
    for mutate in range(-1,16):
        for fail in range(-1,16):
            for item in range(3):
                for kinetic in [False,True]:yield row(mutate_at=mutate,fail=fail,after=state(item=item,kinetic=kinetic,durations=[9,5]))
    rng=random.Random(0x5553455354415254)
    def random_state():return state(item=rng.randrange(3),remaining=rng.choice(ints),virtual_remaining=rng.choice(ints),kinetic=bool(rng.randrange(2)),using=bool(rng.randrange(2)),off=bool(rng.randrange(2)),hand=rng.randrange(2),hands=[rng.randrange(3),rng.randrange(3)],level=rng.randrange(2),clients=[bool(rng.randrange(2)),bool(rng.randrange(2))],durations=[rng.choice(ints),rng.choice(ints)])
    for _ in range(12000):yield row(start_hand=rng.randrange(2),shared_type=bool(rng.randrange(2)),vibrations=[bool(rng.randrange(2)),bool(rng.randrange(2))],kinetic_items=[bool(rng.randrange(2)),bool(rng.randrange(2))],initial=random_state(),after=random_state(),fail=rng.choice([-1,-1,*range(16)]),mutate_at=rng.randrange(-1,16))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-use-start-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemUseStartOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_use_start_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemUseStartOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_use_start_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item use start output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual inherited LivingEntity startUsingItem, native private equipment synchronization and ClientboundSetEquipmentPacket construction, item duration dispatch, using/offhand flag ordering, native UseEffects vibration and KineticWeapon presence. Covers captured held-stack/level/hand arguments, callback mutations/failures, empty stacks, client/server queries and recent-kinetic-map reset. Flag writes and delivery services are controlled; synchronized-data storage, wire encoding, kinetic combat, full inventory/equipment and game tick integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item use start parity passed: {report_path}')


if __name__=='__main__':main()
