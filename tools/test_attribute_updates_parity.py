#!/usr/bin/env python3
"""Compare native attribute-update consequences composed with notifications and health storage."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
try:
    from . import lab, test_rust_parity as base, test_effect_notifications_parity as notifications
except ImportError:
    import lab
    import test_rust_parity as base
    import test_effect_notifications_parity as notifications


def corpus():
    def d(v):return int.from_bytes(struct.pack('>d',v),'big')
    def f(v):return int.from_bytes(struct.pack('>f',v),'big')
    def decorate(row):
        row['health']=[f(20),f(30)];row['servers']=[True,False]
        for after,c in enumerate([row['initial'],row['after']]):
            c.update(data=after,absorption=f(12) if not after else None,maximums=[d(10),d(6)] if not after else [d(.5),d(2)],direct_attributes=after,managers=[0,1] if not after else [1,0],values=[[d(1)]*5,[d(-1)]*5])
        return row
    for row in notifications.corpus():yield decorate(row)
    def op(name,a=0,**kw):
        v=dict(op=name,a=a,b=0,source=-1,partial=0x3f000000,list=[4,5],reapply=True);v.update(kw);return v
    def fresh():
        row=decorate(next(notifications.corpus()));row.update(setup=[],ops=[op('attribute-update'),op('pack',0),op('pack',1)],fail=-1,mutate=-1,mutations=[])
        return row
    doubles=[-float('inf'),-1.,-0.,0.,1e-50,.5,1.,10.,20.,1e50,float('inf'),float('nan')]
    floats=[-float('inf'),-1.,-0.,0.,.5,10.,20.,float('inf'),float('nan')]
    for kind in [0,1]:
        for value in floats:
            for maximum in doubles:
                row=fresh();row['health']=[f(value),f(value)];row['initial'].update(absorption=f(value),maximums=[d(maximum),d(maximum)]);row['ops'][0]=op('attribute-update',kind);yield row
    for kind in range(5):
        for servers in [[False,False],[True,False],[False,True],[True,True]]:
            for fail in range(-1,8):
                for mutate in range(-1,8):
                    row=fresh();row.update(servers=servers,fail=fail,mutate=mutate);row['ops'][0]=op('attribute-update',kind);yield row
    for value in doubles:
        for level in [0,1]:
            row=fresh();row.update(servers=[True,True]);row['initial'].update(level=level,values=[[d(value)]*5,[d(value)]*5]);row['ops'][0]=op('attribute-update',3);yield row
    rng=random.Random(0x41545452555044)
    for _ in range(500):
        row=fresh();row.update(health=[f(rng.choice(floats)) for _ in range(2)],servers=[bool(rng.randrange(2)) for _ in range(2)],fail=rng.randrange(-1,20),mutate=rng.randrange(-1,20),ops=[op(rng.choice(['attribute-update','refresh','notify-update','notify-remove','pack']),rng.randrange(2)) for _ in range(8)])
        for c in [row['initial'],row['after']]:
            c.update(data=rng.randrange(2),absorption=f(rng.choice(floats)),maximums=[d(rng.choice(doubles)) for _ in range(2)],direct_attributes=rng.randrange(2),managers=[rng.randrange(2) for _ in range(2)],values=[[d(rng.choice(doubles)) for _ in range(5)] for _ in range(2)],holders=[rng.randrange(5) for _ in range(3)],level=rng.randrange(2))
        row['setup']=[op('pending-add',0,b=i) for i in range(3)];yield row


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-attribute-updates-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaAttributeUpdatesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_effect_notifications_parity.py",lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/attribute_updates_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaAttributeUpdatesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';enriched_hash=base.source_hashes([enriched])
        run([binary,'attribute_updates_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';rust_output=build/f'rust-{side}.txt';matched=base.compare(reference,rust_output)
        base.ensure_sources_unchanged([enriched],enriched_hash)
        tested_fixtures.update(enriched_hash);fixture_paths.append(enriched)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'rust_output_sha256':lab.digest(rust_output,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} attribute update output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Inherited LivingEntity.onAttributeUpdated composed with native effect notifications, pending sets and synchronized health storage. Includes max-health and max-absorption lowering, repeated maximum reads, Float dirty callbacks, dimension refresh dispatch, server waypoint manager selection, direct attribute-map field capture and positive-range tracking. Tests finite/nonfinite numeric values, callback mutation/failure prefixes and packet notifications. Actual attribute values, dimension rebuild and waypoint-manager internals remain controlled; registered modifier bodies and complete player/entity gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped attribute update parity passed: {report_path}')


if __name__=='__main__':main()
