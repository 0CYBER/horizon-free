#!/usr/bin/env python3
"""Compare default fastutil ObjectOpenHashSet storage and live iterator behavior."""
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
    rng=random.Random(0x4f50454e534554)
    def op(kind,k=-1):return [kind,k]
    def hashes(mode,n):
        def unmix(x):
            h=((x^(x>>16))*pow(0x9e3779b9,-1,2**32))&0xffffffff
            return h if h<0x80000000 else h-2**32
        if mode==0:return [0]*n
        if mode==1:return [unmix(31)]*n
        if mode==2:return [unmix(127+i%4) for i in range(n)]
        if mode==3:return list(range(n))
        return [rng.randrange(-2**31,2**31) for _ in range(n)]
    for mode in range(5):
        for n in [0,1,7,23,24,25,47,48,49,64,97,128]:
            for order in range(4):
                h=hashes(mode,n);ids=list(range(n));rng.shuffle(ids)
                if order%2:ids.append(-1)
                ops=[op('add',i) for i in ids]+[op('cursor')]
                for i in range(len(ids)):
                    ops.append(op('next'))
                    if i%2==0 or order>=2:ops.append(op('delete'))
                ops.extend([op('force-next'),op('delete')])
                for i in reversed(ids):ops.extend([op('contains',i),op('get',i),op('remove',i),op('add',i)])
                rng.shuffle(ids);ops.extend(op('remove',i) for i in ids)
                ops.extend([op('cursor'),op('clear'),op('next'),op('force-next'),op('delete')])
                yield dict(hashes=h,ids=list(range(n)),ops=ops)
    for mode in range(5):
        for n in [1,8,24,25,49,128]:
            h=hashes(mode,n)
            # Equal keys in distinct objects must retain the original member.
            yield dict(hashes=h+h,ids=list(range(n))*2,ops=[op('add',i) for i in range(n)]+[op('add',i+n) for i in range(n)]+[op('get',i+n) for i in range(n)]+[op('remove',i+n) for i in range(n)])
    for _ in range(600):
        n=rng.choice([16,32,64,128]);h=hashes(rng.randrange(5),n);ids=list(range(n));rng.shuffle(ids)
        ops=[op('add',i) for i in ids]+[op('cursor')]
        # External mutation is deliberately not followed by iterator.remove:
        # that unsupported combination can corrupt native size metadata.
        for __ in range(300):
            k=rng.choice(['add','add','remove','contains','get','cursor','next','force-next','clear'])
            if k=='clear' and rng.randrange(20):k='remove'
            ops.append(op(k,rng.randrange(-1,n)))
        yield dict(hashes=h,ids=list(range(n)),ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-object-open-set-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaObjectOpenSetOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/object_open_set_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaObjectOpenSetOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'object_open_set_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} object open set output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'fastutil_sha256':{str(p.relative_to(lab.ROOT)):lab.digest(p,'sha256') for p in libraries if 'fastutil' in str(p)},'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Pinned fastutil 8.5.18 default ObjectOpenHashSet used for pending attribute updates. Compares add/contains/get/remove/clear, null membership, equal distinct object retention, slot iteration, growth/shrink, iterator removal with wrapped clusters and live external mutation prefixes. Key equality/hash methods are pure. Alternate constructors, collection bulk methods and external structural mutation followed by iterator removal are outside this scoped gate; attribute/player integration remains pending.'}
    lab.write_json(report_path,report);print(f'Scoped object open set parity passed: {report_path}')


if __name__=='__main__':main()
