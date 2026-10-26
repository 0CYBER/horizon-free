#!/usr/bin/env python3
"""Compare native SimpleContainer transfers, shared stack identity and change callbacks."""
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
    kinds=['set','add','remove','remove-type','no-update','clear','all','can-add','get','alias','copy','count','component']
    edges=[-2147483648,-1,0,1,2,63,64,65,99,100,2147483647]
    def op(kind,**kw):
        v=dict(kind=kind,a=0,b=3,slot=0,amount=1,item=1,max=99,fail_at=0,mutate_at=0,mutate_ref=0,after_count=2,after_max=8,mutate_slot=1,replace_ref=2,key=0,value=64);v.update(kw);return v
    def stack(item,count):return dict(item=item,count=count,pop=7,patch=[])
    def row(ops,slots=None,count=5):return dict(prototypes=[[],[[0,64]],[[0,1],[1,0]]],stacks=[stack(1,count),stack(1,60),stack(2,1),stack(-1,0)],slots=[1,3,3] if slots is None else slots,ops=ops)
    for kind in kinds:
        for count in edges:
            for amount in edges:
                yield row([op(kind,amount=amount),op('can-add'),op('get',slot=0),op('count',a=3,amount=3)],count=count)
    for kind in ['set','add','remove','remove-type','no-update','get']:
        for slot in [-2147483648,-1,0,1,2,3,2147483647]:
            for slots in [[],[0],[0,0,0],[1,3,3]]:
                for fail in range(1,5):
                    for mutate in [0,1,2]:
                        yield row([op(kind,slot=slot,fail_at=fail,mutate_at=mutate,amount=99)],slots=slots)
    # Transfer limits differ from canAddItem; the first empty slot clears the
    # remaining copy before setItem clamps it. Include zero/negative limits.
    for limit in edges:
        for size in edges:
            for slots in [[3,3],[1,3],[1,1],[0,0]]:
                v=row([op('can-add'),op('add',max=limit)],slots=slots,count=100)
                v['prototypes'][1]=[[0,size]]
                yield v
    rng=random.Random(0x434f4e5441494e)
    def integer():return rng.choice(edges) if rng.randrange(3) else rng.randrange(-2147483648,2147483648)
    def patch():return [[rng.randrange(3),rng.choice([None,integer()])] for _ in range(rng.randrange(5))]
    for _ in range(2200):
        prototypes=[[[k,integer()] for k in rng.sample(range(3),rng.randrange(4))] for __ in range(3)]
        stacks=[dict(item=rng.randrange(-1,3),count=integer(),pop=integer(),patch=patch()) for __ in range(4)]
        slots=[rng.randrange(4) for __ in range(rng.choice([0,1,3,6]))]
        ops=[op(rng.choice(kinds),a=rng.randrange(4),b=rng.randrange(4),slot=rng.randrange(-1,len(slots)+2),amount=integer(),item=rng.randrange(3),max=integer(),fail_at=rng.randrange(5),mutate_at=rng.randrange(4),mutate_ref=rng.randrange(4),after_count=integer(),after_max=integer(),mutate_slot=rng.randrange(-1,len(slots)+1),replace_ref=rng.randrange(4),key=rng.randrange(3),value=rng.choice([None,integer()])) for __ in range(16)]
        yield dict(prototypes=prototypes,stacks=stacks,slots=slots,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-simple-container-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSimpleContainerOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/simple_container_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaSimpleContainerOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'simple_container_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} simple container output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual SimpleContainer get/set/add/removal, reverse remove-by-type, clear/remove-all, can-add and count queries over real shared ItemStack storage. Small native prototypes expose component equality and copy isolation; alias identities and raw/visible state are traced. Native size/change callbacks cover mutations and failures before/after storage writes. Rust uses production reference-counted stack storage and container methods. Persistence, recipe accounting, arbitrary getItem/setItem overrides, player inventory/menu transactions, full component values and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped simple container parity passed: {report_path}')


if __name__=='__main__':main()
