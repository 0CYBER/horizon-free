#!/usr/bin/env python3
"""Compare native PlayerInventory transfers, shared stack identity and change callbacks."""
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
    kinds=['get','set','remove','no-update','remove-ref','select','selected','set-selected','remove-selected','pick','add-pick','clear','changed','replace','equip','get-equip','alias','copy','count','component']
    edges=[-2147483648,-1,0,1,2,8,9,35,36,39,40,41,42,43,64,2147483647]
    def op(kind,**kw):
        v=dict(kind=kind,inventory=0,other=1,a=0,b=3,slot=0,amount=1,all=False,equipment=0,key=0,value=64);v.update(kw);return v
    def stack(item,count,patch=None):return dict(item=item,count=count,pop=7,patch=patch or [])
    def row(ops,count=5,slots=None,selected=0):return dict(prototypes=[[],[[0,64]],[[0,1],[1,0],[2,100]]],stacks=[stack(1,count),stack(1,60),stack(2,1),stack(-1,0)],equipment_order=list(range(7)),shared_equipment=False,inventories=[dict(selected=selected,changed=2147483647,mainhand=0,slots=slots if slots is not None else [[0,0],[8,1],[35,2],[36,2],[40,0],[42,1]]),dict(selected=8,changed=0,mainhand=-1,slots=[[2,1],[39,0]])],ops=ops)
    for kind in kinds:
        for slot in edges:
            for amount in [-2147483648,-1,0,1,64,2147483647]:
                yield row([op(kind,slot=slot,amount=amount),op('changed'),op('copy'),op('count',a=3,amount=3)])
    for enchanted in [False,True]:
        for selected in range(9):
            for free in [-1,0,1,8,9,35]:
                for kind in ['pick','add-pick','replace','remove-ref','remove-selected']:
                    slots=[[i,1] for i in range(43) if i!=free]
                    v=row([op(kind,slot=35),op('replace',other=0)],slots=slots,selected=selected)
                    if enchanted:v['stacks'][1]['patch']=[[4,1]]
                    yield v
    rng=random.Random(0x504c41594552494e)
    def integer():return rng.choice(edges) if rng.randrange(2) else rng.randrange(-2147483648,2147483648)
    def value(k):return 0 if k==3 else rng.randrange(2) if k==4 else integer()
    def patch():return [[k,rng.choice([None,value(k)])] for k in rng.sample(range(6),rng.randrange(7))]
    for _ in range(1500):
        prototypes=[[[k,value(k)] for k in rng.sample(range(6),rng.randrange(7))] for __ in range(3)]
        stacks=[dict(item=rng.randrange(-1,3),count=integer(),pop=integer(),patch=patch()) for __ in range(4)]
        inventories=[dict(selected=rng.randrange(9),changed=integer(),mainhand=rng.randrange(-1,4),slots=[[i,rng.randrange(4)] for i in rng.sample(range(43),rng.randrange(44))]) for __ in range(2)]
        ops=[]
        for __ in range(18):
            k=rng.randrange(6)
            ops.append(op(rng.choice(kinds),inventory=rng.randrange(2),other=rng.randrange(2),a=rng.randrange(4),b=rng.randrange(4),slot=rng.choice(edges) if rng.randrange(2) else rng.randrange(43),amount=integer(),all=bool(rng.randrange(2)),equipment=rng.randrange(8),key=k,value=rng.choice([None,value(k)])))
        yield dict(prototypes=prototypes,stacks=stacks,inventories=inventories,equipment_order=rng.sample(range(7),7),shared_equipment=bool(rng.randrange(2)),ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-player-inventory-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerInventoryOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/player_inventory_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaPlayerInventoryOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'player_inventory_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} player inventory output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual player Inventory storage, equipment, selected slots, hotbar picking, matching/crafting eligibility, reference removals, clear/replacement, counts and change counter. Actual ItemStack components cover damage, unbreakable, one enchantment and custom names; fixture-controlled equipment-map iteration order isolates native JVM Map.of random ordering. Shared stack and equipment references are traced across two inventories, including self-replacement and failures after selection changes. Pickup, item/equipment ticks, menus, recipes, persistence, arbitrary virtual overrides, full components and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped player inventory parity passed: {report_path}')


if __name__=='__main__':main()
