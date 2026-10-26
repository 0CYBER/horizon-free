#!/usr/bin/env python3
"""Compare entity and boat passenger fluid-interaction bounds with vanilla."""
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
    def bits(v):return struct.pack('>d',float(v)).hex()
    def row(**kw):
        f=dict(actor=[-.3,0,-.3,.3,1.8,.3],boat=[-1,0,-1,1,.6,1],input=[-.3,0,-.3,.3,1.8,.3],status=0,vehicle=2,direct=False,fail=-1,mutate_at=-1,after_actor=[7,8,9,10,11,12],after_boat=[-1,0,-1,1,2,1],after_status=1,modified=None)
        f.update(kw)
        for k in ['actor','boat','input','after_actor','after_boat','modified']:
            if f[k] is not None:f[k]=[bits(v) for v in f[k]]
        return f
    values=[-float('inf'),-1e30,-2,-.001,-0.0,0.0,1e-40,.0005,.001,.0015,.002,.5,.6,1.7989999999999997,1.799,1.7990000000000002,1.8,2,1e30,float('inf'),float('nan')]
    for top in values:
        for low in values:
            for status in range(-1,5):
                for direct in [False,True]:yield row(boat=[-1,top,-1,1,top,1],actor=[-.3,low,-.3,.3,1.8,.3],input=[-.3,low,-.3,.3,1.8,.3],status=status,direct=direct)
    for component in range(6):
        for value in values:
            b=[-.3,0,-.3,.3,1.8,.3];b[component]=value
            for vehicle in range(4):
                for fail in [-1,0,1,2]:
                    for mutate in [-1,0,1]:yield row(actor=b,vehicle=vehicle,fail=fail,mutate_at=mutate,modified=[1,2,3,4,5,6])
    rng=random.Random(0x464c554944424f58)
    for _ in range(6000):
        yield row(actor=[rng.choice(values) for _ in range(6)],input=[rng.choice(values) for _ in range(6)],boat=[rng.choice(values) for _ in range(6)],after_boat=[rng.choice(values) for _ in range(6)],vehicle=rng.randrange(4),status=rng.randrange(-1,5),after_status=rng.randrange(-1,5),direct=bool(rng.randrange(2)),fail=rng.choice([-1,-1,0,1,2]),mutate_at=rng.choice([-1,-1,0,1,2]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-fluid-box-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFluidBoxOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/fluid_box_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFluidBoxOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'fluid_box_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} fluid-box output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.getFluidInteractionBox and AbstractBoat.modifyPassengerFluidInteractionBox. Captured deflated entity bounds, virtual vehicle lookup/modifier dispatch, actual boat status predicate, clipping/null results at the boat waterline, AABB endpoint normalization, nonfinite/signed-zero arithmetic, callback mutations and failures. Entity/vehicle snapshots are controlled; live passenger lifecycle and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped fluid box parity passed: {report_path}')


if __name__=='__main__':main()
