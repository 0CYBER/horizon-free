#!/usr/bin/env python3
"""Compare registered health attributes, synchronization and hunger regeneration with vanilla."""
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


def corpus(catalog):
    def bits(v):return struct.pack('>f',v).hex()
    def double(v):return struct.pack('>d',v).hex()
    def op(mode,value=0,attribute=0,amount=0,operation=0,**kw):
        o=dict(mode=mode,value=bits(value),attribute=attribute,amount=double(amount),operation=operation,id='horizon:a',nutrition=2);o.update(kw);return o
    def food(n=20,s=5,e=0,t=9):return dict(food=n,saturation=bits(s),exhaustion=bits(e),timer=t)
    def row(entity,ops,**kw):
        f=dict(entity=entity,ops=ops,health=bits(10),absorption=bits(0),dead=False,regen=True,difficulty=2,food=food(),fail=-1);f.update(kw);return f
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,5,10,20,1e30,float('inf'),float('nan')]
    entities=list(catalog['entities'])
    for entity in entities:
        for value in values:
            ops=[op('hurt'),op('heal',1),op('base',amount=value),op('heal',1),op('set',value),op('base',attribute=1,amount=value),op('absorption',value),op('reset'),op('tick'),op('pack')]
            for operation in range(3):ops.extend([op('add',amount=.25,operation=operation),op('heal',1),op('tick'),op('pack')])
            ops.extend([op('remove'),op('heal',1),op('dead')]);yield row(entity,ops)
    for entity in entities:
        for n,s,t in [(20,5,9),(18,0,79),(0,0,79)]:
            for fail in [-1,0,1,2]:yield row(entity,[op('tick') for _ in range(100)]+[op('pack')],food=food(n,s,0,t),fail=fail)
    rng=random.Random(0x4845414c464f4f44)
    for _ in range(1500):
        ops=[op(rng.choice(['hurt','dead','heal','set','absorption','tick','tick','eat','exhaust','pack','base','add','remove','reset']),rng.choice(values),rng.randrange(2),rng.choice(values),rng.randrange(3),id=rng.choice(['horizon:a','horizon:b']),nutrition=rng.choice([-1,0,1,2,20])) for __ in range(rng.randrange(5,80))]
        yield row(rng.choice(entities),ops,health=bits(rng.choice(values)),absorption=bits(rng.choice(values)),dead=bool(rng.randrange(2)),regen=bool(rng.randrange(2)),difficulty=rng.randrange(4),food=food(rng.choice([0,17,18,19,20]),rng.choice(values),rng.choice(values),rng.choice([0,9,79])),fail=rng.choice([-1,-1,0,1,2,5]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-attribute-health-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaAttributeHealthOracle.java'
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/attribute_health_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaAttributeHealthOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'catalog.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'catalog.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'attribute_health_fixture',build],'rust');run([binary,'attribute_health_fixture',build,'live'],'rust-live');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        live_matched=base.compare(reference,build/'rust-live.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched,'live_matched_output_lines':live_matched})
        print(f'{side}: {matched:,} attribute health output lines match both Rust storage backends',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Registered entity AttributeMap defaults/modifiers composed with actual LivingEntity/Player health methods, SynchedEntityData float health and FoodData regeneration. All 93 default suppliers; attribute mutations, health limits, actual healing and dirty state, hunger/exhaustion/timers, observer failures and starvation requests. Internal absorption storage and starvation damage delivery are controlled; Player absorption synchronization, attribute-change refresh, full damage and live gameplay remain pending.'}
    report['live_rust_output_sha256']=lab.digest(build/'rust-live.txt','sha256')
    report['scope'] += ' Both the per-attribute EntityAttributes storage and native-order LiveAttributes component execute the same production health binding. This health trace normalizes attribute collection order; actual JVM map/queue order is covered by the separate live-attribute gate.'
    lab.write_json(report_path,report);print(f'Scoped attribute health parity passed: {report_path}')


if __name__=='__main__':main()
