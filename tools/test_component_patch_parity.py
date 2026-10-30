#!/usr/bin/env python3
"""Compare native component patch storage, ordered mutations and shared copies."""
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
    def op(kind,a=0,b=0,**kw):return dict(kind=kind,a=a,b=b,**kw)
    values=[-2147483648,-1,0,1,2147483647,'00000000','80000000','7fc00000','7fa12345','ff800000','3f800000']
    for initial in values:
        for first in [None,*values]:
            for second in [None,*values]:
                yield dict(prototype=[[0,initial]],ops=[op('set',key=0,value=first),op('copy',b=1),op('save',b=0),op('set',key=0,value=second),op('remove',key=0),op('restore',a=2,b=0),op('set',a=2,key=0,value=second)])
    yield dict(prototype=[],ops=[op('build',entries=[[0,None],[1,2],[0,None]]),op('restore'),op('set',key=0,value=1),op('clear'),op('from'),op('set',key=0,value=1)])
    rng=random.Random(0x434f4d504f4e454e)
    def val():return rng.choice(values) if rng.randrange(2) else (rng.randrange(-2147483648,2147483648) if rng.randrange(2) else f'{rng.getrandbits(32):08x}')
    for _ in range(3000):
        prototype=[[k,val()] for k in rng.sample(range(16),rng.randrange(8))]
        ops=[]
        for __ in range(rng.randrange(10,31)):
            kind=rng.choice(['set','set','remove','copy','save','apply','restore','clear','build','from']);a=rng.randrange(3);b=rng.randrange(3)
            if kind=='set':v=op(kind,a,b,key=rng.randrange(16),value=rng.choice([None,val()]))
            elif kind=='remove':v=op(kind,a,b,key=rng.randrange(16))
            elif kind=='build':v=op(kind,a,b,entries=[[rng.randrange(16),rng.choice([None,val()])] for ___ in range(rng.randrange(20))])
            else:v=op(kind,a,b)
            ops.append(v)
        yield dict(prototype=prototype,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-component-patch-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaComponentPatchOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/component_patch_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaComponentPatchOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'component_patch_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} component patch output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual PatchedDataComponentMap reads, set/remove return values, ordered patch iteration/key sets, apply versus unsanitized restore, fromPatch sanitation, saved patches and independent map copies. Immutable prototypes have at most seven keys so native builder order is defined without identity-hash tables. Integer and boxed-float values cover NaN/signed zero; generic Rust callers must supply native value equality and prototype order. Includes mutation-before-error for removal markers without prototype defaults. Component codecs, prototype hash-table construction, object identity hashes, mutable payload aliases and full item/inventory integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped component patch parity passed: {report_path}')


if __name__=='__main__':main()
