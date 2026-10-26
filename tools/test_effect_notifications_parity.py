#!/usr/bin/env python3
"""Compare native effect eligibility, notification bodies, pending sets and passenger packets."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    rng=random.Random(0x4e4f54494659)
    durations=[-2147483648,-2,-1,0,1,20,200,2147483647]
    amps=[-1,0,1,2,255,256,2147483647]
    floats=[0,0x80000000,0x3f000000,0x3f800000,0xbf800000,0x7f800000,0x7fc00000]
    def state(**kw):
        v=dict(key=4,duration=20,amplifier=1,ambient=False,visible=True,icon=True,hidden=-1,factor=0x3f000000,previous=0);v.update(kw);return v
    def config(**kw):
        v=dict(states=[state(key=i) for i in range(6)],level=0,client=False,removed=False,dirty=False,tags=[False]*3,attributes=0,routes=[0,1],holders=[0,1,2],passengers=[0,2,1,0],connections=[0,1],entity=42);v.update(kw);return v
    def op(name,a=4,b=0,**kw):
        v=dict(op=name,a=a,b=b,source=-1,partial=0x3f000000,list=[4,5],reapply=True);v.update(kw);return v
    def row(**kw):
        v=dict(hashes=[0,1,2,3,0,0],attribute_hashes=[0,31,64],initial=config(),after=config(states=[state(key=i,duration=-1,amplifier=9,ambient=True,visible=False,icon=False) for i in range(6)],attributes=1,routes=[1,0],holders=[2,0,1],passengers=[1,0],connections=[1,0],entity=-13,dirty=None),setup=[op('pending-add',0,i) for i in range(3)]+[op('pending-add',1,2)],ops=[op('add'),op('force',5),op('all')],fail=-1,mutate=-1,mutations=[op('put',4,5)]);v.update(kw);return v
    for bits in range(8):
        for effect in range(6):
            for mutate in range(-1,3):
                for fail in range(-1,3):
                    yield row(initial=config(tags=[bool(bits&(1<<i)) for i in range(3)]),ops=[op('eligible',effect)],mutate=mutate,fail=fail)
    for mode in ['notify-add','notify-update','notify-remove','send','refresh','add','force','remove','all']:
        for mutate in range(-1,15):
            for fail in range(-1,15):
                yield row(setup=[op('put',4,4),op('put',5,5),op('pending-add',0,0),op('pending-add',0,1),op('pending-add',1,2)],ops=[op(mode),op('get')],mutate=mutate,fail=fail)
    for mode in ['notify-add','notify-update','notify-remove','send','refresh']:
        for client in [False,True]:
            for reapply in [False,True]:
                for passengers in [[],[2],[0],[1,0,1],[0,2,1,0]]:
                    for connections in [[0,1],[1,0],[-1,1],[0,-1]]:
                        yield row(initial=config(client=client,passengers=passengers,connections=connections),ops=[op(mode,reapply=reapply)])
    for count in [0,1,3,24,25,49,64]:
        for collision in [False,True]:
            for mutate in range(-1,8):
                for changes in [[],[op('pending-clear',0)],[op('pending-remove',0,0)],[op('pending-add',0,-1)]]:
                    cfg=config(holders=[i%4 for i in range(max(1,count))]);cfg2=config(holders=[(i+1)%4 for i in range(max(1,count))],attributes=1,routes=[1,0],dirty=None)
                    yield row(attribute_hashes=[0 if collision else i*31 for i in range(max(1,count))],initial=cfg,after=cfg2,setup=[op('pending-add',0,i) for i in reversed(range(count))],ops=[op('refresh')],mutate=mutate,mutations=changes)
    for _ in range(500):
        count=rng.choice([6,8,16]);nattrs=rng.choice([3,8,24,49])
        def states():return [state(key=rng.randrange(count),duration=rng.choice(durations),amplifier=rng.choice(amps),ambient=bool(rng.randrange(2)),visible=bool(rng.randrange(2)),icon=bool(rng.randrange(2)),hidden=rng.randrange(-1,i),factor=rng.choice(floats),previous=rng.choice(floats)) for i in range(count)]
        def cfg():return config(states=states(),level=rng.randrange(2),client=bool(rng.randrange(2)),removed=bool(rng.randrange(2)),dirty=rng.choice([False,True,None]),tags=[bool(rng.randrange(2)) for _ in range(3)],attributes=rng.randrange(2),routes=[rng.randrange(2) for _ in range(2)],holders=[rng.randrange(4) for _ in range(nattrs)],passengers=[rng.randrange(3) for _ in range(rng.randrange(6))],connections=[rng.randrange(-1,2) for _ in range(2)],entity=rng.randrange(-2**31,2**31))
        def mutation():
            k=rng.choice(['put','drop','clear','copy','alias','pending-add','pending-remove','pending-clear'])
            return op(k,rng.randrange(2) if k.startswith('pending') else rng.randrange(count),rng.randrange(-1,nattrs) if k.startswith('pending') else rng.randrange(count))
        yield row(hashes=[rng.choice([0,0,64,i*31]) for i in range(count)],attribute_hashes=[rng.choice([0,31,64,i*31]) for i in range(nattrs)],initial=cfg(),after=cfg(),setup=[op('put',rng.randrange(count),rng.randrange(count)) for _ in range(count)]+[op('pending-add',rng.randrange(2),rng.randrange(-1,nattrs)) for _ in range(nattrs)],ops=[op(rng.choice(['eligible','notify-add','notify-update','notify-remove','send','refresh','add','force','remove','all','get','blend','copy','alias']),rng.randrange(count),rng.randrange(count),reapply=bool(rng.randrange(2)),list=[rng.randrange(count) for _ in range(rng.randrange(5))],partial=rng.choice(floats)) for _ in range(15)],fail=rng.randrange(-1,30),mutate=rng.randrange(-1,30),mutations=[mutation() for _ in range(3)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-effect-notifications-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEffectNotificationsOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/effect_notifications_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEffectNotificationsOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';enriched_hash=base.source_hashes([enriched])
        run([binary,'effect_notifications_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';rust_output=build/f'rust-{side}.txt';matched=base.compare(reference,rust_output)
        base.ensure_sources_unchanged([enriched],enriched_hash)
        tested_fixtures.update(enriched_hash);fixture_paths.append(enriched)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'rust_output_sha256':lab.digest(rust_output,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} effect notification output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Inherited LivingEntity eligibility and add/update/remove notification bodies, private refreshDirtyAttributes and sendEffectToPassengers, composed with native active-effect storage. Actual effect instances, fastutil pending sets, immutable passenger lists and update/remove packet constructors are used. Covers eligibility tag precedence, dirty writes, captured maps/sets/connections, callback mutation, pending iteration/clear prefixes, packet fields and failures. Registered modifier bodies, onAttributeUpdated consequences and packet transport remain controlled; full entity/player lifecycle and gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped effect notification parity passed: {report_path}')


if __name__=='__main__':main()
