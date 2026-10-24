#!/usr/bin/env python3
"""Compare player crouching edge-backoff probes and lazy guards with vanilla."""
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
    def row(flags=6,mover='SELF',movement=(.3,-.08,.2),step=.6,fall=0,clear=None,fail=-1,bounds=(-.3,1,-.3,.3,2.8,.3)):
        return dict(flying=bool(flags&1),staying=bool(flags&2),ground=bool(flags&4),mover=mover,movement=list(map(bits,movement)),step=single(step),fall=bits(fall),clear=clear if clear is not None else [True]*30,fail=fail,box=list(map(bits,bounds)))
    for flags in range(8):
        for mover in ['SELF','PLAYER','PISTON','SHULKER','SHULKER_BOX']:
            for y in [-.1,-0.0,0,.1,float('nan')]:
                for fall in [0,.2,.6,1,float('nan')]:
                    for clear in [[],[False],[True],[False]+[True]*30]:yield row(flags,mover,(.3,y,.2),fall=fall,clear=clear)
    values=[-.6,-.1,-math.nextafter(.05,1),-.05,-math.nextafter(.05,0),-1e-9,-0.0,0,1e-9,math.nextafter(.05,0),.05,math.nextafter(.05,1),.1,.6,float('nan'),float('inf')]
    for x in values:
        for z in values:
            for clear in [[],[True]*30,[False,True,False]+[True]*10,[True,False,True,False]+[True]*10]:yield row(movement=(x,-.08,z),clear=clear)
    for fail in range(-1,35):
        for flags in [0,2,6,7]:yield row(flags=flags,clear=[False]+[True]*40,fail=fail)
    rng=random.Random(0x45444745)
    for _ in range(6000):
        low=[rng.choice([-16,-1,-0.0,0,.1,1,1e8,float('nan')]) for _ in range(3)];high=[v+rng.choice([0,1e-8,.6,1.8]) for v in low]
        yield row(flags=rng.randrange(8),mover=rng.choice(['SELF','SELF','PLAYER','PISTON']),movement=[rng.choice(values),rng.choice([-.1,0,.1,float('nan')]),rng.choice(values)],step=rng.choice([0,.5,.6,1,float('nan'),float('inf')]),fall=rng.choice([0,.2,.6,1,float('nan')]),clear=[bool(rng.randrange(2)) for _ in range(50)],fail=rng.choice([-1,-1,0,1,2,3,10]),bounds=(*low,*high))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-edge-backoff-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEdgeBackoffOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/edge_backoff_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEdgeBackoffOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'edge_backoff_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} edge-backoff output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Player.maybeBackOffFromEdge, isAboveGround and canFallAtLeast with controlled step/staying/ground getters and ordered collision-free query results. Compares mover and flight guards, fall-distance boundaries, float step widening, exact thin probe bounds, X/Z/diagonal reductions, signed zero/nonfinite inputs and injected query errors. This does not establish live player crouching state, attribute provisioning or integrated registered-world ledge behavior; those need separate integration checks.'}
    lab.write_json(report_path,report);print(f'Scoped edge backoff parity passed: {report_path}')


if __name__=='__main__':main()
