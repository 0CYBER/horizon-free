#!/usr/bin/env python3
"""Compare native PlayerEquipment transfers, shared stack identity and change callbacks."""
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
    kinds=['get-hand','set-hand','get-item','set-item','set-equipment','clear-equipment','clear','select','get','set','count','alias','copy']
    def op(kind,**kw):
        v=dict(kind=kind,a=0,b=3,slot=0,hand=0,equipment=0,amount=1,fail=False,mutate=False,mutate_ref=0,after_count=2,after_selected=8,replace_slot=0,replace_ref=1);v.update(kw);return v
    def row(ops,**kw):
        v=dict(stacks=[dict(item=1,count=2,pop=5),dict(item=2,count=1,pop=2),dict(item=1,count=1,pop=0),dict(item=-1,count=0,pop=0)],slots=[[0,0],[1,0],[8,1],[35,2]],equipment=[[0,0],[1,1],[7,2]],raw_entries=[],selected=0,ops=ops);v.update(kw);return v
    for kind in kinds:
        for equipment in range(8):
            for count in edges:
                for hand in [0,1]:
                    v=row([op(kind,equipment=equipment,hand=hand,amount=count),op('select',slot=8),op('get-hand',hand=0)])
                    v['stacks'][0]['count']=count
                    yield v
    for kind in ['set-item','set-hand']:
        for equipment in range(8):
            for hand in [0,1]:
                for fail in [False,True]:
                    for mutate in [False,True]:
                        for selected in [-1,0,8,9]:
                            yield row([op(kind,equipment=equipment,hand=hand,fail=fail,mutate=mutate,after_selected=selected),op('get-hand',hand=0)])
    for raw in [[],[[0,0]],[[0,3]]]:
        yield row([op('clear-equipment'),op('set-equipment',equipment=0),op('clear-equipment'),op('clear')],raw_entries=raw)
    rng=random.Random(0x45515549504d454e)
    for _ in range(1600):
        yield row([op(rng.choice(kinds),a=rng.randrange(4),b=rng.randrange(4),slot=rng.choice([-1,0,8,9,35,36,40,42,43]),hand=rng.randrange(2),equipment=rng.randrange(8),amount=rng.choice(edges),fail=bool(rng.randrange(2)),mutate=bool(rng.randrange(2)),mutate_ref=rng.randrange(4),after_count=rng.choice(edges),after_selected=rng.choice([-1,0,8,9]),replace_slot=rng.choice([-1,0,8,40]),replace_ref=rng.randrange(4)) for __ in range(15)],stacks=[dict(item=rng.randrange(-1,3),count=rng.choice(edges),pop=rng.choice(edges)) for __ in range(4)],slots=[[i,rng.randrange(4)] for i in rng.sample(range(43),rng.randrange(44))],equipment=[[i,rng.randrange(4)] for i in rng.sample(range(8),rng.randrange(9))],raw_entries=[[0,rng.randrange(4)]] if rng.randrange(2) else [],selected=rng.randrange(9))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-player-equipment-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerEquipmentOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/player_equipment_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaPlayerEquipmentOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'player_equipment_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} player equipment output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual PlayerEquipment get/set/isEmpty and inherited LivingEntity hand/slot access over a real Inventory and shared ItemStack references. Equipment changes write storage before the controlled onEquipItem callback; mutations/failures and main-hand selection changes retain native prefixes. Raw enum-map main-hand entries exercise base storage independently from the selected main-hand override. Full onEquipItem effects, synchronized flags, active-use integration and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped player equipment parity passed: {report_path}')


if __name__=='__main__':main()
