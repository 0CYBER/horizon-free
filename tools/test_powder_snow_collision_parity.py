#!/usr/bin/env python3
"""Compare contextual powder-snow collision selection with pinned vanilla."""
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
    modes=['empty','position','player','marker','falling','factory_player','factory_marker','placement','placement_null','with_player','with_marker','with_falling','with_null']
    distances=[-math.inf,-1.0,-0.0,0.0,math.nextafter(2.5,-math.inf),2.5,math.nextafter(2.5,math.inf),100.0,math.inf,math.nan]
    def row(mode,y,block_y,fall,tag,item,count,descending,fail=-1):
        return {'mode':mode,'y':bits(y),'other':bits(y),'descending':descending,'always':True,'item':0,'count':0,'stand':False,'fail':False,'powder':[block_y,bits(fall),tag,item,count,fail]}
    for block_y in [-2147483648,0,2147483647]:
        threshold=(block_y+1.0)-epsilon
        heights=[math.nextafter(threshold,-math.inf),threshold,math.nextafter(threshold,math.inf),block_y+1.0,block_y-0.5,block_y+2.0,math.inf,math.nan]
        for mode in modes:
            for fall in distances:
                for tag in [False,True]:
                    for item,count in [(0,0),(4,0),(4,1),(4,-1),(1,1)]:
                        for descending in [False,True]:
                            for y in heights:
                                yield row(mode,y,block_y,fall,tag,item,count,descending)
    for mode in modes:
        for fall in [0.0,2.5,3.0]:
            for tag in [False,True]:
                for item in [1,4]:
                    for descending in [False,True]:
                        for fail in [0,1,2]:
                            yield row(mode,1.0,0,fall,tag,item,1,descending,fail)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-powder-snow-collision-parity';build.mkdir(parents=True,exist_ok=True)
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
        print(f'{side}: {matched:,} powder-snow-collision output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'block_states':1,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Native PowderSnowBlock contextual collision and canEntityWalkOnPowderSnow decisions with controlled entity metadata, entity-tag membership and equipment snapshots. Checks empty/position/player/nonliving/falling-block/placement contexts, fall-distance and top-height adjacent doubles, nonfinite values, descending, boot identity/count and lazy tag/equipment callbacks with injected failures. Full selected shape geometry and query traces match. Entity tag loading, entity lifecycle, freezing/sinking and live movement remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped powder-snow collision parity passed: {report_path}')


if __name__=='__main__':main()
