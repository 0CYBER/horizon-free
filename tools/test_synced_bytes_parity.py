#!/usr/bin/env python3
"""Compare multiple synchronized byte fields, assignment and reentrant callbacks with vanilla."""
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
    def bits(v):return f'{v&255:02x}'
    def value(id=8,v=0):return dict(id=id,value=bits(v))
    def op(mode='set',id=8,v=0,**kw):
        o=dict(mode=mode,id=id,value=bits(v),force=False,values=[],hook='none',other=17,other_value=bits(7),other_force=False,fail=-1,mask=1,flag=True);o.update(kw);return o
    def row(initial,ops):return dict(initial=initial,ops=ops)
    for start in range(256):
        for mask in [-2147483648,-1,0,1,2,3,4,128,255,256,2147483647]:
            for flag in [False,True]:
                yield row([value(17),value(8,start)],[op('flag',mask=mask,flag=flag),op('defaults'),op('pack'),op('flag',mask=mask,flag=flag)])
    for mode in ['set','assign','flag']:
        for hook in ['none','pack','set','assign','flag']:
            for fail in range(-1,6):
                for other in [8,17]:
                    yield row([value(8),value(17)],[op(v=3),op(mode,v=5,values=[value(8,5),value(17,7),value(8,3)],hook=hook,other=other,mask=129,flag=False,fail=fail),op('pack'),op('defaults')])
    rng=random.Random(0x53594e4342595445)
    for _ in range(2000):
        ids=[8]+rng.sample([i for i in range(64) if i!=8],rng.randrange(0,10));initial=[value(id,rng.randrange(256)) for id in ids];ops=[]
        for __ in range(rng.randrange(5,35)):
            ops.append(op(rng.choice(['set','set','get','pack','defaults','assign','flag']),rng.choice(ids+[255]),rng.randrange(256),force=bool(rng.randrange(2)),values=[value(rng.choice(ids+[255]),rng.randrange(256)) for ___ in range(rng.randrange(6))],hook=rng.choice(['none','pack','set','assign','flag']),other=rng.choice(ids),other_value=bits(rng.randrange(256)),other_force=bool(rng.randrange(2)),mask=rng.randrange(-2147483648,2147483648),flag=bool(rng.randrange(2)),fail=rng.choice([-1,-1,-1,0,1,2,3])))
        yield row(initial,ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-synced-bytes-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSyncedBytesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/synced_bytes_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaSyncedBytesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'synced_bytes_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} synchronized byte output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual byte SynchedEntityData values and inherited LivingEntity using-item/offhand queries plus protected flag writes. Covers every initial byte, signed masks, ordered dirty/default output, assignment, reentrant flag/data writes, callback packing and failures before dirty marking. Rust shares a typed scalar implementation with existing synchronized floats. Heterogeneous serializer maps, entity schema/network codecs and full item-use/gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped synchronized byte parity passed: {report_path}')


if __name__=='__main__':main()
