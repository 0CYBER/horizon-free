#!/usr/bin/env python3
"""Compare block-definition flags and calculated solidity with pinned vanilla."""
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
    def box(values):return [struct.pack('>d',float(v)).hex() for v in values]
    shapes=[[],[[0,0,0,1,1,1]],[[0,0,0,0.1,1,0.1]],[[0,0,0,1,0.1,1]],[[0,0,0,0.01,0.01,0.01],[0.99,0.99,0.99,1,1,1]]]
    threshold=0.7291666666666666
    for x in [0.0,0.125,0.5,threshold,1.0,2.0]:
        for y in [0.0,1e-8,0.125,0.5,math.nextafter(1.0,0.0),1.0,math.nextafter(1.0,2.0)]:
            z=3*threshold-x-y
            for extent in [math.nextafter(z,-math.inf),z,math.nextafter(z,math.inf)]:shapes.append([[0,0,0,x,y,extent]])
    rng=random.Random(0x534f4c4944)
    for _ in range(1000):
        shape=[]
        for _ in range(rng.randrange(1,5)):
            low=[rng.uniform(-1,1) for _ in range(3)];high=[v+rng.choice([1e-8,1e-7,0.125,0.5,1.0,2.0]) for v in low];shape.append([*low,*high])
        shapes.append(shape)
    for shape in shapes:
        for on in [False,True]:
            for off in [False,True]:
                for cached in [False,True]:
                    for occludes in [False,True]:yield {'shape':[box(v) for v in shape],'on':on,'off':off,'cached':cached,'occludes':occludes}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-block-properties-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockPropertiesOracle.java'
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java'
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/block_properties_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBlockPropertiesOracle',mode,path],side+'-'+mode)
    java('server','export',build)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'block-registry.nbt',build/'cached-collision.nbt',build/'block-properties.nbt'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'block_properties_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} block-properties output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'registry_states':35723,'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'All pinned block-definition force-solid, replaceability, liquid and occlusion flags and raw occlusion geometry are imported privately; Rust computes legacy solidity from cached collision shapes and solid rendering from occlusion geometry. All six native face-occlusion full-shape tests are compared for every state and every synthetic shape/occlusion-flag case; Rust caches shared shape masks and retains typed errors for unknown state IDs. Actual native BlockState isSolid, liquid, canOcclude, isSolidRender and no-context canBeReplaced are compared for every state, together with raw cached bounds and synthetic force-flag/cache/shape threshold combinations using the private native solidity calculator and native full-shape test. Item/fluid replacement, block support, survival, dynamic collision and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped block-properties parity passed: {report_path}')


if __name__=='__main__':main()
