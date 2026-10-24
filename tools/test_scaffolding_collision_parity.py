#!/usr/bin/env python3
"""Compare contextual scaffolding collision selection with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    import math
    def bits(value):return struct.pack('>d',value).hex()
    epsilon=struct.unpack('>f',struct.pack('>f',1e-5))[0]
    positions=[-2147483648,-64,-1,0,1,320,2147483647]
    modes=['empty','position','player','marker','placement','placement_null','with_player','with_marker','with_null']
    for block_y in positions:
        heights=[-math.inf,math.inf,math.nan,block_y-0.5,block_y+0.5,block_y+1.5]
        for top in [0.0,1.0]:
            threshold=(block_y+top)-epsilon
            heights += [math.nextafter(threshold,-math.inf),threshold,math.nextafter(threshold,math.inf),block_y+top]
        for mode in modes:
            for i,y in enumerate(heights):
                for descending in [False,True]:
                    for distance in range(8):
                        for bottom in [False,True]:
                            for water in [False,True]:
                                yield {'mode':mode,'y':bits(y),'other':bits(y),'descending':descending,'always':bool(i%2),'item':2,'count':1,'stand':False,'fail':False,'scaffolding':[distance,bottom,water,block_y]}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-scaffolding-collision-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionContextOracle.java'
    helpers=[oracle.with_name('VanillaFaceSupportOracle.java')]
    runner=lab.ROOT/'tools/test_collision_context_parity.py'
    sources=[oracle,*helpers,runner,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/collision_context_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaCollisionContextOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'collision_context_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} scaffolding-collision output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'block_states':32,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual CollisionContext.getCollisionShape dispatch and native scaffolding behavior for all 32 scaffolding states, nine context variants, descending flags, integer Y extremes, nonfinite positions and adjacent double values around the top/bottom float-epsilon thresholds. Full shape coordinates, extrema, box ordering and cube metadata match. A rejecting world proxy verifies selection performs no world reads. Scaffolding placement/update/falling behavior and live movement remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped scaffolding collision parity passed: {report_path}')


if __name__=='__main__':main()
