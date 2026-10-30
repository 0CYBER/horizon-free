#!/usr/bin/env python3
"""Compare native use remainders and strict scalar stack validation."""
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
    edges=[-2147483648,-1,0,1,2,64,99,2147483647]
    def op(kind,**kw):
        v=dict(kind=kind,a=0,b=1,count=1,item=1,patch=[],prototype=[],template=0,infinite=False,fail=0,extra_count=2,source_count=3,replace=-1);v.update(kw);return v
    def row(item=1,count=1,ops=None):
        return dict(prototypes=[[],[[0,64]],[[0,1],[1,10],[2,0]]],stacks=[dict(item=item,count=count,patch=[]),dict(item=2,count=1,patch=[]),dict(item=-1,count=0,patch=[])],ops=ops or [])
    for count in edges:
        for before in edges:
            for item in [-1,0,1,2]:
                for infinite in [False,True]:
                    for patch in [[],[[0,1],[1,10]],[[0,0]],[[0,99]]]:
                        yield row(item,count,[op('new',count=2,patch=patch,b=0),op('convert',count=before,infinite=infinite)])
    for fail in range(3):
        for replace in [-1,0,1,2]:
            for extra_count in edges:
                for source_count in edges:
                    yield row(1,1,[op('new',b=0),op('convert',count=2,fail=fail,replace=replace,extra_count=extra_count,source_count=source_count),op('convert',count=3)])
    rng=random.Random(0x52454d41494e4445)
    for _ in range(2400):
        ops=[op('new',b=0,count=rng.choice([n for n in edges if n]),item=rng.choice([1,2]),patch=[[0,rng.choice(edges)]])]
        for __ in range(rng.randrange(2,8)):
            ops.extend([op('count',a=rng.randrange(3),count=rng.choice(edges)),op('convert',a=rng.randrange(3),b=rng.randrange(3),count=rng.choice(edges),infinite=bool(rng.randrange(2)),fail=rng.randrange(3),replace=rng.randrange(-1,3),source_count=rng.choice(edges),extra_count=rng.choice(edges))])
        yield row(rng.randrange(-1,3),rng.choice(edges),ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-use-remainder-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaUseRemainderOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/use_remainder_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaUseRemainderOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'use_remainder_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} use remainder output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual UseRemainder.convertIntoRemainder composed with native ItemStackTemplate creation/validation and shared ItemStack references. Covers finite/infinite materials, visible count comparisons, exhausted and empty-but-distinct inputs, invalid templates returning canonical EMPTY, extra-item callbacks with mutations/failures, and captured-reference identity. Rust uses its item-template factory with scalar validation; contained component validation, concrete actor inventory insertion/drop, item finish side effects and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped use remainder parity passed: {report_path}')


if __name__=='__main__':main()
