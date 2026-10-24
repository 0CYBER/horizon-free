#!/usr/bin/env python3
"""Compare contextual liquid collision selection with pinned vanilla."""
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
    modes=['empty','position','player','marker','factory_player','factory_marker','placement','placement_null','with_player','with_marker','with_null']
    def row(mode,y,lava,level,block_y,above,shape,stand,fail,always=False):
        return {'mode':mode,'y':bits(y),'other':bits(y),'descending':True,'always':always,'item':0,'count':0,'stand':stand,'fail':False,'liquid':[level,lava,block_y,above,shape,fail]}
    # Every liquid block level and context, including the always-fluid bypass.
    for lava in [False,True]:
        for level in range(16):
            for mode in modes:
                for always in [False,True]:
                    for block_y in [-2147483648,0,2147483647]:
                        for shape in range(4):
                            for fail in [-1,0,1,2]:
                                yield row(mode,block_y+2.0,lava,level,block_y,0,shape,True,fail,always)
    # Exact top boundaries and all source/flowing fluid states above the block.
    for lava in [False,True]:
        for mode in ['player','placement','with_player']:
            for block_y in [-64,0,2147483647]:
                for shape,top in enumerate([-math.inf,1.0,0.5,1.5]):
                    threshold=(block_y+top)-epsilon
                    heights=[math.nextafter(threshold,-math.inf),threshold,math.nextafter(threshold,math.inf),math.inf,math.nan]
                    for y in heights:
                        for above in range(-1,32):
                            for stand in [False,True]:
                                yield row(mode,y,lava,0,block_y,above,shape,stand,-1)
    # Inject failures after height checks on an allowed candidate.
    for mode in ['player','placement','with_player']:
        for lava in [False,True]:
            for above in range(-1,32):
                for fail in [0,1,2,3]:
                    yield row(mode,4.0,lava,0,0,above,1,True,fail)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-liquid-collision-parity';build.mkdir(parents=True,exist_ok=True)
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
        print(f'{side}: {matched:,} liquid-collision output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'block_states':32,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Native contextual LiquidBlock collision dispatch for all 32 water/lava states. Checks the always-fluid bypass, source-only selection, living-entity gating, supplied entity standing shapes, exact height boundaries, all 33 above-block fluid states, callback order and failure propagation, null/placement contexts and wrapping above-block coordinates. Full selected shape data and callback traces are compared. Entity-specific standing shape implementations, liquid simulation and live movement remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped liquid collision parity passed: {report_path}')


if __name__=='__main__':main()
