#!/usr/bin/env python3
"""Compare native item cooldownss and strict scalar stack validation."""
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
    edges=[-2147483648,-2147483647,-1,0,1,2,20,2147483646,2147483647]
    floats=[0,0x80000000,1,0x3d4ccccc,0x3d4ccccd,0x3f000000,0x3f800000,0xbf800000,0x4b7fffff,0x4b800000,0x4effffff,0x7f800000,0xff800000,0x7fc00000,0x7fa12345]
    groups=['test:'+''.join('an' if i&(1<<k) else 'c0' for k in range(6)) for i in range(64)]+['minecraft:stick','minecraft:shield','minecraft:air',':empty','a:','',':','test:path/with.-_09']
    def op(kind,**kw):
        v=dict(kind=kind,group=0,duration=1,partial=0,seconds=0x3f800000,player=True,fail=-1,mutate=-1,action='none',other=1,after=1);v.update(kw);return v
    def row(clock=0,initial=None,ops=None,**kw):
        v=dict(groups=groups,clock=clock,initial=initial or [],item=1,count=1,component=True,seconds=0x3f800000,group=0,ops=ops or []);v.update(kw);return v
    for clock in edges:
        for duration in edges:
            for partial in floats:
                yield row(clock,[[0,duration]],[op('percent',partial=partial),op('duration',seconds=partial),op('tick'),op('percent',partial=partial)])
    for action in ['add','remove','clock','tick','stack']:
        for mutate in [0,1]:
            for fail in [-1,0,1,2,3]:
                for other in [0,1,16]:
                    for after in [-2147483648,-1,0,1,2147483647]:
                        yield row(0,[[i,0] for i in range(16)],[op('tick',action=action,mutate=mutate,fail=fail,other=other,after=after),op('percent')])
    for item in range(3):
        for count in edges:
            for component in [False,True]:
                for player in [False,True]:
                    yield row(ops=[op('apply',player=player,seconds=0x3f000000,mutate=0,action='stack',other=1,after=2),op('percent')],item=item,count=count,component=component)
    rng=random.Random(0x434f4f4c444f574e)
    for _ in range(2400):
        initial=[[rng.randrange(len(groups)),rng.choice(edges)] for __ in range(rng.randrange(40))]
        ops=[op(rng.choice(['add','add-stack','remove','tick','percent','apply','duration','clock']),group=rng.randrange(len(groups)),duration=rng.choice(edges),partial=rng.choice(floats),seconds=rng.randrange(0x100000000),player=bool(rng.randrange(2)),fail=rng.randrange(-1,4),mutate=rng.randrange(-1,3),action=rng.choice(['add','remove','clock','tick','stack','none']),other=rng.randrange(len(groups)),after=rng.choice(edges)) for __ in range(rng.randrange(3,15))]
        yield row(rng.choice(edges),initial,ops,item=rng.randrange(3),count=rng.choice(edges),component=bool(rng.randrange(2)),group=rng.randrange(-1,len(groups)))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-cooldowns-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemCooldownsOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_cooldowns_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemCooldownsOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_cooldowns_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item cooldowns output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual ItemCooldowns storage/add/remove/tick/percentage and default stack group resolution, plus UseCooldown ticks/apply through real Player class selection and controlled getCooldowns/start/end callbacks. Includes Identifier hashing/path-first collision ordering, tree-bin expiry, wrapping times, float bits, empty component visibility, callback mutation/failure and nested ticks. Rust composes checked collection iteration with shared stack components. Complete finish-use integration, server cooldown packets and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item cooldowns parity passed: {report_path}')


if __name__=='__main__':main()
