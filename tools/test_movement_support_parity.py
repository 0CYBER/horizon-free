#!/usr/bin/env python3
"""Compare support positions and Entity block movement factors with vanilla."""
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
    def single(v):return struct.pack('>f',float(v)).hex()
    def state(flags,jump=1,speed=1):return dict(fence=bool(flags&1),wall=bool(flags&2),gate=bool(flags&4),water=bool(flags&8),bubble=bool(flags&16),jump=single(jump),speed=single(speed))
    def row(mode=0,offset=0.2,position=(0.3,1.5,-0.3),support=(17,-64,-33),states=None,fail=-1):
        return dict(mode=mode,offset=single(offset),position=list(map(bits,position)),cached=[23,75,-81],support=support,states=states or [state(0)],fail=fail)
    offsets=[-float('inf'),-1,-0.0,0,1e-6,1e-5,struct.unpack('>f',bytes.fromhex('3727c5ad'))[0],0.2,0.5,0.500001,1,float('inf'),float('nan')]
    coords=[-float('inf'),-2147483649,-64,-0.5,-0.0,0,0.2,0.5,0.500001,1,1.5,2147483648,float('inf'),float('nan')]
    for flags in range(8):
        for offset in offsets:
            for y in coords:
                for support in [None,[17,-64,-33]]:yield row(offset=offset,position=(y,y,-y),support=support,states=[state(flags)])
    for mode in range(1,6):
        for flags in range(32):
            for support in [None,[17,-64,-33]]:
                for factor in [-1,-0.0,0,0.4,1,2,float('nan'),float('inf')]:
                    yield row(mode=mode,support=support,states=[state(flags,factor,factor),state(flags^7,.8,.6),state(0,.25,.125)])
    for mode in [0,3,4,5]:
        for flags in range(32):
            for fail in range(-1,14):yield row(mode=mode,states=[state(flags),state(flags^7,.8,.6),state(0,.25,.125)],fail=fail)
    rng=random.Random(0x535550504f5254)
    for _ in range(5000):
        yield row(mode=rng.randrange(6),offset=rng.choice(offsets),position=[rng.choice(coords) for _ in range(3)],support=rng.choice([None,[17,-64,-33],[2147483647,-2147483648,2147483647]]),states=[state(rng.randrange(32),rng.choice([0,.5,1,2]),rng.choice([0,.5,1,2])) for _ in range(3)],fail=rng.choice([-1,-1,-1,0,1,2,5,8,12]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-movement-support-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMovementSupportOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/movement_support_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaMovementSupportOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'movement_support_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} movement-support output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.getOnPos(float), all three public support-position wrappers, getBlockJumpFactor and getBlockSpeedFactor. Controlled block providers vary fence/wall tags, gate type, water/bubble identities and float factors. Compares lazy query order, cached block position, support-coordinate retention, float offset widening, floor behavior, nonfinite inputs and failures. Jump queries the lower block unconditionally; speed can short-circuit. This does not establish registered movement property provisioning, LivingEntity movement efficiency, live entity storage or full gameplay.'}
    lab.write_json(report_path,report);print(f'Scoped movement support parity passed: {report_path}')


if __name__=='__main__':main()
