#!/usr/bin/env python3
"""Compare LivingEntity landing wrapper and base fall callbacks with vanilla."""
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
    def bits(v): return struct.pack('>d',float(v)).hex()
    def row(**kw):
        f=dict(fall=6,dy=-.08,ground=True,water=False,after_water=False,support=None,after_support=None,mutate=False,fail=-1,server=True,fluid_mutate=False,fluid_result=False,changed_mutate=False,particle_mutate=False,block_mutate=False,after_fall=12,safe=3,air=False,position=[.3,1.5,-.3],block_position=[0,1,-1],landing=[3,4,5])
        f.update(kw)
        for k in ['fall','dy','after_fall','safe']:f[k]=bits(f[k])
        f['position']=[bits(v) for v in f['position']]
        return f
    values=[-float('inf'),-1e40,-1,-1e-46,-0.0,0,1e-46,2.9999989,2.999999,3,3.999999,4,4.000001,6,7,37.5,100,1e40,float('inf'),float('nan')]
    for fall in values:
        for safe in values:
            for server in [False,True]:
                for ground in [False,True]:
                    for water in [False,True]:
                        for after_water in [False,True]:
                            for air in [False,True]:yield row(fall=fall,safe=safe,server=server,ground=ground,water=water,after_water=after_water,air=air)
    positions=[[.3,1.5,-.3],[3.5,0,5.5],[3.5,0,10],[8,0,5.5],[4.5,0,6.5],[float('nan'),1,5.5],[3.5,float('inf'),float('nan')],[float('inf'),0,-float('inf')],[30000000.5,-64,-30000000.5],[-0.0,0,-0.0]]
    for position in positions:
        for landing in [[3,4,5],[0,1,-1],[30000000,-64,-30000000],[-2147483648,0,2147483647]]:
            for block_position in [landing,[0,0,0]]:
                for block_mutate in [False,True]:yield row(position=position,landing=landing,block_position=block_position,block_mutate=block_mutate)
    for field in ['fluid_mutate','changed_mutate','particle_mutate','block_mutate','mutate']:
        for after in values:
            for fail in range(-1,14):yield row(**{field:True},after_fall=after,fail=fail,support=[17,-64,-33],after_support=[31,15,-1])
    rng=random.Random(0x4c414e44)
    flags=['ground','water','after_water','mutate','server','fluid_mutate','fluid_result','changed_mutate','particle_mutate','block_mutate','air']
    for _ in range(12000):
        f={k:bool(rng.randrange(2)) for k in flags}
        for k in ['fall','dy','after_fall','safe']:f[k]=rng.choice(values) if rng.randrange(3)==0 else rng.uniform(-5,80)
        yield row(**f,position=rng.choice(positions),support=rng.choice([None,[17,-64,-33]]),after_support=rng.choice([None,[31,15,-1]]),fail=rng.choice([-1,-1,*range(14)]))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-landing-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingLandingOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_landing_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingLandingOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_landing_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living-landing output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity.checkFallDamage including its base Entity fall handling. Controlled fluid update, onChangedBlock, attribute, state, particle and block landing providers exercise callback order, mutations, server class identity, particle positions/counts, nonfinite arithmetic, climbing cleanup and partial state after failures. Full fluid immersion, enchantment hooks, particle rendering and player/world integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living landing parity passed: {report_path}')


if __name__=='__main__':main()
