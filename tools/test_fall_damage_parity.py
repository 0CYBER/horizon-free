#!/usr/bin/env python3
"""Compare base Entity fall accumulation and landing callbacks with vanilla."""
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
    def row(fall=2,dy=-.08,ground=True,water=False,support=None,after_support=None,mutate=False,fail=-1):return dict(fall=bits(fall),dy=bits(dy),ground=ground,water=water,support=support,after_support=after_support,mutate=mutate,fail=fail)
    values=[-float('inf'),-1e40,-1,-.08,-1e-46,-0.0,0,1e-46,.08,1,1e40,float('inf'),float('nan')]
    for fall in values:
        for dy in values:
            for ground in [False,True]:
                for water in [False,True]:
                    for support in [None,[17,-64,-33]]:yield row(fall,dy,ground,water,support)
    for fall in [-1,0,2,float('nan')]:
        for dy in [-1,-1e-46,0,1]:
            for support in [None,[17,-64,-33]]:
                for after in [None,[31,15,-1]]:
                    for mutate in [False,True]:
                        for fail in range(-1,6):yield row(fall,dy,support=support,after_support=after,mutate=mutate,fail=fail)
    rng=random.Random(0x46414c4c)
    for _ in range(5000):
        fall=rng.uniform(-.5,10);dy=rng.uniform(-2,1)
        if rng.randrange(4)==0:fall=rng.choice(values)
        if rng.randrange(4)==0:dy=rng.choice(values)
        yield row(fall,dy,bool(rng.randrange(2)),bool(rng.randrange(2)),rng.choice([None,[17,-64,-33]]),rng.choice([None,[31,15,-1]]),bool(rng.randrange(2)),rng.choice([-1,-1,0,1,2,3,4,5]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-fall-damage-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFallDamageOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/fall_damage_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFallDamageOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'fall_damage_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} fall-damage output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual base Entity.checkFallDamage with controlled water state, block landing callback, supporting-block lookup, HIT_GROUND emission and reset. Compares float-narrowed downward displacement accumulation, grounded branches, state/position/support mutations during landing, callback order, nonfinite arithmetic and partial state after failures. LivingEntity fluid updates/particles, registered Block.fallOn behavior, health/damage/attributes and the full movement lifecycle are not established by this scoped check.'}
    lab.write_json(report_path,report);print(f'Scoped fall damage parity passed: {report_path}')


if __name__=='__main__':main()
