#!/usr/bin/env python3
"""Compare world-border extents, tick interpolation and collision geometry with pinned vanilla."""
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
    partials=[struct.pack('>f',v).hex() for v in [0,-0.0,0.25,0.5,1,-1,2,float('nan'),float('inf')]]
    def row(ops,center=0,size=10):
        points=[0,-0.0,-1,1,-5,5,-6,6,center,center-size/2,center+size/2,math.nextafter(center+size/2,-float('inf')),float('nan'),float('inf')]
        queries=[]
        for i,x in enumerate(points):
            z=0 if i%2 else x
            for margin in [0,1,-0.5]:
                width=[0,0.6,1,3][i%4]
                queries.append(list(map(bits,[x,z,margin,0,0,0,width,1.8,width])))
        return dict(ops=ops,partials=partials,queries=queries)
    for center in [0,-0.0,0.25,-0.25,29999984,-29999984,1.7e308]:
        for size in [0,-0.0,1e-8,0.125,1,5.5,59999968,1.7e308]:
            for limit in [0,1,8,29999984,2147483647]:
                yield row([['center',bits(center),bits(-center)],['size',bits(size)],['limit',limit]],center,size)
    for start,target in [(0,10),(10,0),(3.25,17.75),(59999968,0),(0,59999968),(1.7e308,0),(0,1.7e308),(0,-0.0)]:
        for duration in [0,1,2,7,20,(1<<53)-1,1<<53,(1<<63)-1]:
            for begin in [0,(1<<63)-1,-(1<<63)]:
                ops=[['lerp',bits(start),bits(target),duration,begin],['tick',1],['center',bits(0.25),bits(-0.75)],['tick',1],['limit',8],['tick',2],['tick',4],['tick',12]]
                yield row(ops,0,start)
    rng=random.Random(0x424f52444552)
    for index in range(600):
        center=rng.uniform(-100,100);size=rng.uniform(0,200)
        ops=[['center',bits(center),bits(rng.uniform(-100,100))],['size',bits(size)]]
        for _ in range(12):
            action=rng.randrange(5)
            if action==0:ops.append(['center',bits(rng.uniform(-100,100)),bits(rng.uniform(-100,100))])
            elif action==1:ops.append(['size',bits(rng.uniform(0,200))])
            elif action==2:ops.append(['limit',rng.randrange(0,201)])
            elif action==3:ops.append(['lerp',bits(rng.uniform(0,200)),bits(rng.uniform(0,200)),rng.randrange(0,30),rng.randrange(-(1<<63),(1<<63))])
            else:ops.append(['tick',rng.randrange(0,10)])
        yield row(ops,center,size)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-world-border-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaWorldBorderOracle.java'
    helper=oracle.with_name('VanillaFaceSupportOracle.java')
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/world_border_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaWorldBorderOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'world_border_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} world-border output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual WorldBorder stationary/moving extents, center/absolute-limit updates, tick progression, size/target/time/speed/status, float partial-tick bounds, full collision shape grids and proximity predicates. Covers finite nonnegative sizes, finite centers, nonnegative absolute limits and durations, signed zero, zero and very large durations, wrapping start timestamps, tick completion, retargeting, fractional boundaries and nonfinite predicate/partial-tick inputs. Rust returns typed errors for parameters outside this supported domain. Border saved-data codecs, listeners, dirty-state events, damage, networking and live movement integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped world-border parity passed: {report_path}')


if __name__=='__main__':main()
