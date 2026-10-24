#!/usr/bin/env python3
"""Compare per-tick piston movement limits and accumulator state with vanilla."""
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
    def op(v,time=0,fail=False):return dict(movement=list(map(bits,v)),time=time,fail=fail)
    def row(ops,deltas=(0,0,0),time=0):return dict(ops=ops,deltas=list(map(bits,deltas)),time=time)
    threshold=float(struct.unpack('>f',struct.pack('>f',1e-5))[0])
    values=[-float('inf'),-1,-.51,-.5,-threshold,-0.0,0,threshold,math.sqrt(1e-7),math.nextafter(math.sqrt(1e-7),0),math.nextafter(math.sqrt(1e-7),1),.5,.51,1,float('inf'),float('nan')]
    for x in values:
        for y in values:
            for z in [0,-0.0,.2,float('nan')]:
                for time in [0,1]:yield row([op([x,y,z],time)])
    for axis in range(3):
        for value in values:
            for initial in [0,-.51,.51,.51-threshold,math.nextafter(.51-threshold,0),float('nan'),float('inf')]:
                v=[0,0,0];v[axis]=value;d=[0,0,0];d[axis]=initial
                for fail in [False,True]:yield row([op(v,0,fail),op(v),op(v,1),op(v,1),op([-q for q in v],1)],d)
    for axis in range(3):
        v=[0,0,0];v[axis]=.1;yield row([op(v,0) for _ in range(12)]+[op([-q for q in v],0) for _ in range(12)]+[op(v,1),op(v,-1)])
    rng=random.Random(0x504953544f4e)
    for _ in range(3000):
        ops=[];time=rng.choice([0,1,-1,9223372036854775807,-9223372036854775808])
        for index in range(20):
            if index%5==0:time=rng.choice([0,1,-1,9223372036854775807,-9223372036854775808])
            ops.append(op([rng.choice(values) for _ in range(3)],time,rng.randrange(8)==0))
        yield row(ops,[rng.choice(values) for _ in range(3)],rng.choice([0,1]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-piston-movement-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPistonMovementOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/piston_movement_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaPistonMovementOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'piston_movement_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} piston movement output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.limitPistonMovement over sequential displacement and game-time snapshots. Compares lazy time queries, length and displacement thresholds, per-tick resets, first-nonzero-axis selection, clamped accumulation, signed zero/nonfinite values and state after injected time-query failures. This does not establish piston block event simulation, entity pushing, Entity.move guards or full gameplay.'}
    lab.write_json(report_path,report);print(f'Scoped piston movement parity passed: {report_path}')


if __name__=='__main__':main()
