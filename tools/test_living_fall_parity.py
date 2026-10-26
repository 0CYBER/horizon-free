#!/usr/bin/env python3
"""Compare living fall damage calculation, callbacks and impulse context with vanilla."""
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
    def single(v):
        try:return struct.pack('>f',float(v)).hex()
        except OverflowError:return struct.pack('>f',math.copysign(float('inf'),v)).hex()
    def vector(v):return None if v is None else list(map(bits,v))
    def row(mode=1,distance=10,multiplier=1,safe=3,damage=1,immune=(False,False),impact=None,grace=0,y=0,mutate=False,fail=-1,server=True,hurt_result=True,ops=()):return dict(mode=mode,distance=bits(distance),multiplier=single(multiplier),safe=bits(safe),**{'damage-factor':bits(damage)},immune=list(immune),impact=vector(impact),grace=grace,y=bits(y),mutate=mutate,fail=fail,server=server,hurt_result=hurt_result,ops=list(ops))
    values=[-float('inf'),-1,-0.0,0,2.999999,3,3.999999,4,7,8,10,1e40,float('inf'),float('nan')]
    for mode in [0,1]:
        for distance in values:
            for multiplier in [-1,-0.0,0,.5,1,2,float('nan')]:
                for safe in [0,3,5,float('nan')]:
                    for immune in [(False,False),(True,True),(False,True),(True,False)]:yield row(mode,distance,multiplier,safe,immune=immune)
    for distance in [-0.0,0,4,10,float('nan')]:
        for impact in [None,[0,-1,0],[0,-0.0,0],[0,0,0],[0,2,0],[0,10,0],[0,float('nan'),0]]:
            for grace in [-1,0,1,40]:
                for fail in range(-1,13):yield row(distance=distance,impact=impact,grace=grace,fail=fail,mutate=True)
    rng=random.Random(0x4c4956494e474641)
    for _ in range(4000):yield row(mode=rng.randrange(2),distance=rng.choice(values),multiplier=rng.choice(values),safe=rng.choice(values),damage=rng.choice(values),immune=(bool(rng.randrange(2)),bool(rng.randrange(2))),impact=rng.choice([None,[0,0,0],[1,5,2],[0,float('inf'),0]]),grace=rng.choice([-1,0,1,40]),y=rng.choice(values),mutate=bool(rng.randrange(2)),fail=rng.choice([-1,-1,0,1,2,4,7,10]),server=bool(rng.randrange(2)),hurt_result=bool(rng.randrange(2)))
    for _ in range(2000):
        ops=[]
        for _ in range(20):ops.append(dict(mode=rng.randrange(4),ignore=bool(rng.randrange(2)),position=vector(rng.choice([None,[0,0,0],[17,-64,-33]])),ticks=rng.choice([-2147483648,-1,0,1,40,100,2147483647])))
        yield row(mode=2,impact=rng.choice([None,[1,2,3]]),grace=rng.choice([-1,0,1,40,2147483647]),ops=ops,fail=rng.choice([-1,-1,0,3,7,12]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-fall-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingFallOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_fall_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingFallOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_fall_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living-fall output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity.calculateFallDamage, causeFallDamage and impulse-context setter/reset/grace methods. Controlled immunity/attribute getters, passenger callback, sounds and server hurt callback expose native query order and failure state. Includes changing immunity, passenger mutations, impulse-height limiting, zero/negative/large grace values, float multiplier widening, floor/saturation, signed zero/nonfinite values and setter sequences. Hurt return values do not change the causeFallDamage result. A null level snapshot exercises the native non-server hurt no-op; actual client entity lifecycle, passenger storage/recursion, health/damage processing, attribute provisioning and tick/save/network integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living fall parity passed: {report_path}')


if __name__=='__main__':main()
