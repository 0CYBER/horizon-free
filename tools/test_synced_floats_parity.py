#!/usr/bin/env python3
"""Compare multiple synchronized float fields, assignment and reentrant callbacks with vanilla."""
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
    def bits(v):return struct.pack('>f',v).hex()
    def value(id=9,v=0):return dict(id=id,value=bits(v))
    def op(mode='set',id=9,v=0,**kw):
        o=dict(mode=mode,id=id,value=bits(v),force=False,values=[],hook='none',other=17,other_value=bits(7),other_force=False,fail=-1);o.update(kw);return o
    def row(initial,ops):return dict(initial=initial,ops=ops)
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,7,20,1e30,float('inf'),float('nan')]
    for start in values:
        for v in values:
            for hook in ['none','pack','set','assign']:
                for force in [False,True]:yield row([value(17),value(9,start)],[op(v=v,force=force,hook=hook),op('defaults'),op('pack'),op(v=v,hook=hook),op('assign',values=[value(17,v),value(9,start),value(9,start)],hook=hook),op('pack'),op('defaults')])
    for mode in ['set','assign']:
        for hook in ['none','pack','set','assign']:
            for fail in range(-1,9):
                for other in [9,17]:yield row([value(9),value(17)],[op(v=3),op(mode,v=5,values=[value(9,5),value(17,7)],hook=hook,other=other,fail=fail),op('pack'),op('defaults')])
    for initial in [[],[value(0)],[value(1),value(254)],[value(i,i) for i in range(8)]]:
        yield row(initial,[op('get',255),op('set',255),op('assign',values=[value(v['id'],2) for v in initial]+[value(255)]),op('assign',values=[]),op('pack'),op('defaults')])
    rng=random.Random(0x53594e43464c4f41)
    for _ in range(1500):
        ids=rng.sample(range(64),rng.randrange(1,12));initial=[value(id,rng.choice(values)) for id in ids];ops=[]
        for __ in range(rng.randrange(5,60)):
            ops.append(op(rng.choice(['set','set','get','pack','defaults','assign']),rng.choice(ids+[255]),rng.choice(values),force=bool(rng.randrange(2)),values=[value(rng.choice(ids+[255]),rng.choice(values)) for ___ in range(rng.randrange(6))],hook=rng.choice(['none','pack','set','assign']),other=rng.choice(ids),other_value=bits(rng.choice(values)),other_force=bool(rng.randrange(2)),fail=rng.choice([-1,-1,-1,0,1,2,3])))
        yield row(initial,ops)
    for nan in ['7fa12345','7fc00000','ffa12345']:
        initial=[value(9),value(17)];initial[0]['value']=nan;yield row(initial,[op(v=float('nan')),op('defaults'),op('pack')])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-synced-floats-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSyncedFloatsOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/synced_floats_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaSyncedFloatsOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'synced_floats_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} synchronized float output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual SynchedEntityData Float fields: boxed equality, forced updates, callback-before-dirty flags, ordered dirty/default extraction, received assignment including duplicates, partial unknown-ID/observer failures, batch callbacks and reentrant set/assign/pack. Native table gaps are untouched default dummy Float items; tested keys are explicit configured IDs. Other serializers, type-mismatch errors, wire codecs and player absorption integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped synchronized float parity passed: {report_path}')


if __name__=='__main__':main()
