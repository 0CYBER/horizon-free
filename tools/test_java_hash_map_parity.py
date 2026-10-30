#!/usr/bin/env python3
"""Compare native Java HashMaps and strict scalar stack validation."""
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
    rng=random.Random(0x4a415641484d4150)
    def op(kind,k=0,v=0):return [kind,k,v]
    def hashes(mode,n):
        spread=[0,1,-1,64,128,192,256,512,1024,-2147483648,2147483647]
        def unspread(h):
            h&=0xffffffff;h^=h>>16;return h if h<0x80000000 else h-0x100000000
        if mode==0:return [0]*n
        if mode==1:return list(range(n))
        if mode==2:return [unspread((i%8)*64) for i in range(n)]
        if mode==3:return [unspread(rng.choice(spread)) for _ in range(n)]
        return [rng.randrange(-2147483648,2147483648) for _ in range(n)]
    for mode in range(5):
        for n in [1,7,8,9,10,11,12,13,24,25,48,49,64,97,128]:
            for order in range(6):
                h=hashes(mode,n);ids=list(range(n))
                if order==1:ids.reverse()
                elif order>1:rng.shuffle(ids)
                ops=[op('put',i,i) for i in ids]+[op('cursor')]
                for i in range(n):
                    ops.append(op('next'))
                    if i%2==0:ops.append(op('delete'))
                ops.extend([op('force-next'),op('delete')])
                for i in reversed(ids):ops.extend([op('get',i),op('remove',i),op('put',i,-i)])
                rng.shuffle(ids)
                ops.extend(op('remove',i) for i in ids)
                ops.extend([op('cursor'),op('clear'),op('next'),op('force-next'),op('delete')])
                yield dict(hashes=h,ops=ops)
    kinds=['put','put','remove','get','cursor','next','force-next','delete','clear']
    for _ in range(550):
        n=rng.choice([16,32,64,128]);h=hashes(rng.randrange(5),n);ids=list(range(n));rng.shuffle(ids)
        ops=[op('put',i,i) for i in ids]
        for __ in range(300):
            k=rng.choice(kinds)
            if k=='clear' and rng.randrange(20):k='remove'
            ops.append(op(k,rng.randrange(n),rng.randrange(-2147483648,2147483648)))
        yield dict(hashes=h,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-java-hash-map-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaJavaHashMapOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/java_hash_map_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaJavaHashMapOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'java_hash_map_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} Java HashMap output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'JDK 25 java.util.HashMap default-constructor storage used by vanilla ItemCooldowns, with non-null Comparable keys whose ordering agrees with equality. Direct put/get/remove/clear, linked bucket iteration, collision tree creation/insertion/deletion/splitting, iterator removal and concurrent modification prefixes are compared. Both pinned game classpaths use the same installed JDK implementation; this is a JDK collection comparison, not a gameplay parity test. Null/non-comparable keys, alternate constructors and other Map methods are outside this storage API. Concrete cooldowns, packets and gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped Java HashMap parity passed: {report_path}')


if __name__=='__main__':main()
