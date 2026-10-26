#!/usr/bin/env python3
"""Compare face slicing and support geometry with pinned vanilla."""
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
    def nums(v):return [struct.pack('>d',float(x)).hex() for x in v]
    for cube in [False,True]:
        for mask in range(256):yield {'coords':[nums([0,0.5,1])]*3,'cube':cube,'occupied':[bool(mask&(1<<i)) for i in range(8)]}
    edges=[-1e-7,math.nextafter(-1e-7,-math.inf),0.0,1e-7,math.nextafter(1e-7,math.inf)]
    for lo in edges:
        for hi in [math.nextafter(1-1e-7,-math.inf),1-1e-7,1.0,1+1e-7,math.nextafter(1+1e-7,math.inf)]:
            for axis in range(3):
                coords=[nums([0,1]) for _ in range(3)];coords[axis]=nums([lo,hi]);yield {'coords':coords,'cube':False,'occupied':[True]}
    for boxes in [[],[[0,0,0,1,1,1]],[[0,0,0,1,0.5,1]],[[0,0,0,1,0.25,1],[0,0.75,0,1,1,1]],[[0,0,0,1e-8,1,1]],[[0.4375,0,0.4375,0.5625,0.625,0.5625]],[[0,0,0,1,1,0.125],[0,0,0.875,1,1,1],[0,0,0,0.125,1,1],[0.875,0,0,1,1,1]]]:
        for move in [None,[0,0,0],[1e-8,0,-1e-8],[-17,64,30_000_000]]:
            f={'boxes':[nums(v) for v in boxes]}
            if move is not None:f['move']=nums(move)
            yield f
    rng=random.Random(0x4641434553)
    for i in range(500):
        sizes=[rng.randrange(1,5) for _ in range(3)];cube=i%3==0
        coords=[[j/size for j in range(size+1)] if cube else sorted(rng.sample([-1,-1e-7,0,1e-7,0.25,0.5,0.75,1-1e-7,1,1+1e-7,2],size+1)) for size in sizes]
        yield {'coords':[nums(v) for v in coords],'cube':cube,'occupied':[rng.randrange(4)==0 for _ in range(math.prod(sizes))]}
    for _ in range(500):
        boxes=[]
        for _ in range(rng.randrange(1,5)):
            low=[rng.choice([-1,-1e-7,0,1e-7,0.5,1-1e-7,1,2]) for _ in range(3)]
            boxes.append(nums([*low,*[v+rng.choice([1e-8,1e-7,0.125,0.5,1,2]) for v in low]]))
        yield {'boxes':boxes}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-face-support-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFaceSupportOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/face_support_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFaceSupportOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'face_support_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} face-support output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual VoxelShape.getFaceShape and nested face slicing, shape identity/cache reuse, exact coordinate metadata, extrema, bounds and decomposition, plus FULL/CENTER/RIGID SupportType algorithms over caller-supplied support shapes. Sparse grids, phantom slice bounds, cube/array lookup, epsilon boundaries and translations are included. Per-block support-shape selection, world/entity context, survival and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped face-support parity passed: {report_path}')


if __name__=='__main__':main()
