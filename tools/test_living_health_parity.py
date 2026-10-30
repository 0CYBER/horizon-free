#!/usr/bin/env python3
"""Compare living health, absorption and synchronized Float updates with vanilla."""
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
    def double(v):return struct.pack('>d',v).hex()
    def world(maximum=20,absorption=10,dead=False):return {'max-health':double(maximum),'max-absorption':double(absorption),'dead':dead}
    def op(mode,value=0,force=False):return dict(mode=mode,value=bits(value),force=force)
    def row(health,ops,**kw):
        f=dict(health=[bits(health),bits(7)],absorption=bits(0),world=world(),after=world(5,3,True),fail=-1,mutate_at=-1,ops=ops);f.update(kw);return f
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,5,10,20,1e30,float('inf'),float('nan')]
    for health in values:
        for limit in values:
            for value in values:
                yield row(health,[op('hurt'),op('dead'),op('heal',value),op('pack'),op('set',value),op('base',value),op('absorption',value),op('hurt'),op('dead')],world=world(limit,limit))
    for health in values:
        for value in values:
            for force in [False,True]:yield row(health,[op('sync',value,force),op('sync',value,force),op('pack'),op('pack'),op('sync',health),op('pack')])
    for mode in ['hurt','dead','heal','set','base','absorption','sync']:
        for fail in range(-1,7):
            for mutation in range(-1,7):yield row(10,[op(mode,6),op('pack'),op('heal',1),op('pack')],fail=fail,mutate_at=mutation)
    rng=random.Random(0x4845414c5448)
    for _ in range(1500):
        yield row(rng.choice(values),[op(rng.choice(['hurt','dead','heal','set','base','absorption','sync','pack']),rng.choice(values),bool(rng.randrange(2))) for __ in range(rng.randrange(1,60))],world=world(rng.choice(values),rng.choice(values),bool(rng.randrange(2))),after=world(rng.choice(values),rng.choice(values),bool(rng.randrange(2))),fail=rng.choice([-1,-1,0,3,7,11]),mutate_at=rng.choice([-1,-1,0,2,4,8]))
    for nan in ['7fa12345','7fc00000','ffa12345']:
        f=row(0,[op('sync',float('nan')),op('heal',1),op('pack')]);f['health'][0]=nan;yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-health-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingHealthOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_health_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingHealthOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_health_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living health output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity/Player health, healing, absorption and predicates composed with actual SynchedEntityData Float storage. Checks captured synchronized-data identity, attribute narrowing/clamping, boxed-float equality, callback-before-dirty ordering, partial state after exceptions, mutations and dirty/default extraction. Attribute values and the internal absorption sink are controlled; Player-specific absorption synchronization, full multi-item synchronization, registered attribute binding, damage pipeline and playable entities remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living health parity passed: {report_path}')


if __name__=='__main__':main()
