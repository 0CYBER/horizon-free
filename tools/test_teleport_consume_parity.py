#!/usr/bin/env python3
"""Compare random teleport consume behavior against both pinned JARs."""
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
    floats=[0,0x80000000,1,0x3f000000,0x3f800000,0x41800000,0xc1800000,0x7f7fffff,0x7f800000,0xff800000,0x7fc00000]
    def config(**kw):
        v=dict(position=[1.25,64.5,-2.75],min=-64,height=384,riding=True,success_at=0,move=True,fall=12.5,grace=5,impact=True,random=0,reseed=False,seeds=[99,101]);v.update(kw);return v
    def row(**kw):
        v=dict(initial=config(),after=config(position=[-3.5,71.25,9.5],min=0,height=128,riding=False,success_at=1,fall=7.25,grace=9,random=1,reseed=True),seeds=[0,1],xor=False,fox=False,directional=True,diameter=0x41800000,level=0,fail=-1,mutate=-1);v.update(kw);return v
    for xor in [False,True]:
        for fox in [False,True]:
            for fail in range(-1,19):
                for mutate in range(-1,19):yield row(xor=xor,fox=fox,fail=fail,mutate=mutate)
    for success_at in [-1,0,1,5,14,15,16]:
        for directional in [False,True]:
            for riding in [False,True]:
                for move in [False,True]:yield row(initial=config(success_at=success_at,riding=riding,move=move),directional=directional)
    for diameter in floats:
        for y in [-1e300,-0.0,0.0,1.5,1e300]:
            for min_y in [-2147483648,-64,0,2147483647]:yield row(initial=config(position=[y,y,y],min=min_y,height=2147483647),diameter=diameter)
    rng=random.Random(0x54454c45504f5254)
    def random_config():return config(position=[rng.choice([-1e300,-2147483649.5,-0.0,0.0,1.25,2147483649.5,1e300]) for _ in range(3)],min=rng.choice([-2147483648,-64,-1,0,2147483647]),height=rng.choice([-2147483648,-1,0,1,256,384,2147483647]),riding=bool(rng.randrange(2)),success_at=rng.choice([-1,0,1,3,15,16]),move=bool(rng.randrange(2)),fall=rng.choice([-0.0,1.25,100]),grace=rng.randrange(-1,20),impact=bool(rng.randrange(2)),random=rng.randrange(2),reseed=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)])
    for _ in range(1800):yield row(initial=random_config(),after=random_config(),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],xor=bool(rng.randrange(2)),fox=bool(rng.randrange(2)),directional=bool(rng.randrange(2)),diameter=rng.choice(floats),level=rng.randrange(2),fail=rng.choice([-1,-1,rng.randrange(32)]),mutate=rng.choice([-1,-1,rng.randrange(32)]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-teleport-consume-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaTeleportConsumeOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/teleport_consume_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaTeleportConsumeOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'teleport_consume_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} teleport_consume output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual TeleportRandomlyConsumeEffect with native Legacy/Xoroshiro random streams, ordinary/fox class branches, real fall/impulse reset bodies and BlockUtil offset packing. Compares captured versus live coordinates, repeated min-height reads, logical-height integer overflow, exact candidate bits and next RNG state, 16-attempt exhaustion, dismount/event/sound/directional-particle ordering and callback mutation/failure prefixes. Destination validation/teleport, passenger linkage, world event delivery and full gameplay integration remain controlled or pending.'}
    lab.write_json(report_path,report);print(f'Scoped teleport_consume parity passed: {report_path}')


if __name__=='__main__':main()
