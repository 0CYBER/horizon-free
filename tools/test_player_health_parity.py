#!/usr/bin/env python3
"""Compare registered player health and synchronized absorption with vanilla."""
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
    def op(mode,value=0,**kw):
        o=dict(mode=mode,value=bits(value),amount=double(0),attribute=0,id='horizon:a',operation=0,values=[],fail=-1,pack_callback=False);o.update(kw);return o
    def row(health,absorption,ops,dead=False):return dict(health=bits(health),absorption=bits(absorption),ops=ops,dead=dead)
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,5,10,20,1e30,float('inf'),float('nan')]
    for start in values:
        for maximum in values:
            for value in values:
                yield row(start,start,[op('base',amount=double(maximum)),op('base',attribute=1,amount=double(maximum)),op('set',value),op('absorption',value),op('get-absorption'),op('hurt'),op('dead'),op('pack'),op('heal',value),op('defaults'),op('pack')])
    for fail in range(-1,6):
        for pack in [False,True]:
            ops=[op('base',attribute=1,amount=double(20)),op('set',9),op('absorption',5),op('pack'),op('set',1),op('assign',values=[dict(field=0,value=bits(20)),dict(field=1,value=bits(7)),dict(field=0,value=bits(10))]),op('pack')]
            yield row(1,0,[dict(o,fail=fail,pack_callback=pack) for o in ops])
    rng=random.Random(0x504c415945524850)
    for _ in range(1500):
        ops=[]
        for __ in range(rng.randrange(5,70)):
            ops.append(op(rng.choice(['set','heal','absorption','get-absorption','hurt','dead','base','add','remove','reset','pack','defaults','assign']),rng.choice(values),attribute=rng.randrange(2),amount=double(rng.choice(values)),operation=rng.randrange(3),id=rng.choice(['horizon:a','horizon:b']),values=[dict(field=rng.randrange(2),value=bits(rng.choice(values))) for ___ in range(rng.randrange(5))],fail=rng.choice([-1,-1,-1,0,1,2]),pack_callback=bool(rng.randrange(2))))
        yield row(rng.choice(values),rng.choice(values),ops,bool(rng.randrange(2)))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-player-health-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerHealthOracle.java'
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/player_health_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaPlayerHealthOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'catalog.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'catalog.json',build/'ids.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'player_health_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} player health output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Player health/absorption methods using registered player AttributeMap and one SynchedEntityData table. Exports actual health/absorption field IDs; compares modified attribute limits, healing, player absorption getter/setter, dirty/default extraction, received updates, callback packet drains and failures. Observer side effects are controlled; custom virtual data replacement, other synchronized serializers, wire codecs and full player damage/tick integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped player health parity passed: {report_path}')


if __name__=='__main__':main()
