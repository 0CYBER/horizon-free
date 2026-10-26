#!/usr/bin/env python3
"""Compare native view vectors and blocking-angle arithmetic."""
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
    def single(v):return struct.pack('>f',v).hex()
    def double(v):return struct.pack('>d',v).hex()
    def vec(v):return [double(x) for x in v]
    rng=random.Random(0x41434f53)
    for _ in range(100000):yield dict(mode='acos',value=double(rng.uniform(-1,1)))
    for _ in range(100000):yield dict(mode='acos',value=f'{rng.getrandbits(64):016x}')
    for pivot in [-1,-.5,-0.,0.,.5,1]:
        for direction in [-float('inf'),float('inf')]:
            v=pivot
            for _ in range(256):yield dict(mode='acos',value=double(v));v=math.nextafter(v,direction)
    values=[-float('inf'),-1e30,-720,-360,-180,-90,-45,-1,-0.,0.,1e-40,.1,1,45,90,180,360,720,1e30,float('inf'),float('nan')]
    for pitch in values:
        for yaw in values:yield dict(mode='view',pitch=single(pitch),yaw=single(yaw))
    for _ in range(10000):yield dict(mode='view',pitch=f'{rng.getrandbits(32):08x}',yaw=f'{rng.getrandbits(32):08x}')
    for yaw in values:
        for x in [-1,-1e-5,-0.,0.,1e-6,1e-5,1,float('inf'),float('nan')]:
            for z in [-1,-1e-5,0.,1e-5,1]:yield dict(mode='blocking',source=vec([x,99,z]),position=vec([0,0,0]),yaw=single(yaw))
    for _ in range(12000):
        value=lambda:rng.choice(values) if rng.randrange(4)==0 else rng.uniform(-10000,10000)
        yield dict(mode='blocking',source=vec([value(),value(),value()]),position=vec([value(),value(),value()]),yaw=single(rng.choice(values) if rng.randrange(4)==0 else rng.uniform(-10000,10000)))
    yield dict(mode='fallback')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-view-direction-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaViewDirectionOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/view_direction_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaViewDirectionOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'view_direction_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} view direction output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.calculateViewVector and Java Math.acos, plus the blocking-direction composition of native view vector, Vec3 subtraction/normalization/dot and acos. Exact bit comparisons cover random doubles, inverse-cosine boundaries, float-table directions, nonfinite/zero/near-zero horizontal vectors and the float-PI fallback. Uses pinned pure-Rust libm 0.2.16; full item-blocking dispatch and other platform JVMs remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped view direction parity passed: {report_path}')


if __name__=='__main__':main()
