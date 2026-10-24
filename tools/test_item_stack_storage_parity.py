#!/usr/bin/env python3
"""Compare native item-stack data, copies, split/transmute and consumption."""
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
    kinds=['count','pop','grow','shrink','limit','set','copy','copy-count','copy-clear','split','transmute','consume','consume-return']
    edges=[-2147483648,-2147483647,-1,0,1,2,64,2147483646,2147483647]
    def op(kind,amount=1,**kw):
        v=dict(kind=kind,a=0,b=1,amount=amount,key=0,value=1,item=2,actor=True,fail=False,mutate=False,infinite=False,after_count=2,after_pop=8,after_components=[[0,9],[1,None]]);v.update(kw);return v
    def row(item=1,count=1,ops=None):return dict(prototypes=[[],[[0,2],[1,3]],[[0,8],[2,4]]],stacks=[dict(item=item,count=count,pop=5,patch=[]),dict(item=2,count=2,pop=9,patch=[]),dict(item=-1,count=0,pop=0,patch=[])],ops=ops or [])
    for kind in kinds:
        for item in [-1,0,1,2]:
            for count in edges:
                for amount in edges:yield row(item,count,[op(kind,amount),op('count',1),op('set',value=3)])
    for kind in ['consume','consume-return']:
        for mutate in [False,True]:
            for infinite in [False,True]:
                for fail in [False,True]:
                    for count in edges:yield row(1,count,[op(kind,-1,mutate=mutate,infinite=infinite,fail=fail)])
    rng=random.Random(0x535441434b444154)
    def integer():return rng.choice(edges) if rng.randrange(2) else rng.randrange(-2147483648,2147483648)
    def entries():return [[rng.randrange(4),rng.choice([None,integer()])] for _ in range(rng.randrange(9))]
    for _ in range(3500):
        prototypes=[[[k,integer()] for k in rng.sample(range(4),rng.randrange(5))] for __ in range(3)]
        stacks=[dict(item=rng.randrange(-1,3),count=integer(),pop=integer(),patch=entries()) for __ in range(3)]
        ops=[op(rng.choice(kinds),integer(),a=rng.randrange(3),b=rng.randrange(3),key=rng.randrange(4),value=rng.choice([None,integer()]),item=rng.randrange(3),actor=bool(rng.randrange(2)),fail=bool(rng.randrange(2)),mutate=bool(rng.randrange(2)),infinite=bool(rng.randrange(2)),after_count=integer(),after_pop=integer(),after_components=entries()) for __ in range(rng.randrange(10,25))]
        yield dict(prototypes=prototypes,stacks=stacks,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-stack-storage-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemStackStorageOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_stack_storage_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemStackStorageOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_stack_storage_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item stack storage output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual ItemStack count/pop-time storage, component visibility, copy/copyWithCount/copyAndClear, split, grow/shrink/limit, transmuteCopy, consume and consumeAndReturn. Bound small native prototypes and integer components expose patch sanitation, copies and iterator order. Fixture item-store handles preserve canonical EMPTY aliases, including mutations; callbacks cover creative material queries, original mutation and failures after the returned copy is captured. Count/type/component comparison predicates are compared throughout. Full component codecs, inventory transactions, use/equipment lifecycle and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item stack storage parity passed: {report_path}')


if __name__=='__main__':main()
