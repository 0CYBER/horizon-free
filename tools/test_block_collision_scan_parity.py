#!/usr/bin/env python3
"""Compare lazy block collision scanning and query ordering with pinned vanilla."""
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
    def box(values):return [struct.pack('>d',float(v)).hex() for v in values]
    boxes=[
        [0.2,0.2,0.2,0.8,0.8,0.8],[-0.3,0,0.3,0.3,1.8,0.9],[0,0,0,1,1,1],
        [0.5,0.2,0.2,0.5,0.8,0.8],[0.5,0.2,0.2,0.5+1e-8,0.8,0.8],
        [0.5,0.2,0.2,0.5+2e-7,0.8,0.8],[1-1e-8,0.2,0.2,1+1e-8,0.8,0.8],
        [15.8,63.2,-0.3,16.2,64.8,0.3],[-16.2,-64.3,-16.2,-15.8,-62.5,-15.8],
        [0.5,0.5,0.5,0.5,0.5,0.5],[1e-7,1e-7,1e-7,1-1e-7,1-1e-7,1-1e-7],
        [2,2,2,-1,-1,-1],[-20.2,0.2,0.2,20.2,0.8,0.8],
        [2147483647.2,0.2,0.2,2147483647.8,0.8,0.8],
        [-2147483648.8,-0.5,-0.5,-2147483648.2,0.5,0.5],
        [0.2,2147483647.2,0.2,0.8,2147483647.8,0.8],
        [-0.0,-0.0,-0.0,1e-8,1e-8,1e-8],[-1e-8,-1e-8,-1e-8,0,0,0],
    ]
    palettes=[[shape,large,moving,suffocating] for shape in range(7) for large in [False,True] for moving in [False,True] for suffocating in [False,True]]
    def row(bounds,palette,suffocating=False,missing=0,fail=-1,limit=100000,seed=0):
        return {'box':box(bounds),'palette':palette,'suffocating':suffocating,'missing':missing,'fail':fail,'limit':limit,'seed':seed}
    for entry in palettes:
        for bounds in boxes:
            for suffocating in [False,True]:yield row(bounds,[entry],suffocating)
        for fail in [0,1,2,5,15,50]:
            for suffocating in [False,True]:yield row(boxes[7],[entry],suffocating,fail=fail)
    for missing in [1,2]:
        for bounds in boxes:
            for suffocating in [False,True]:
                for limit in [0,1,4,100000]:yield row(bounds,palettes,suffocating,missing,limit=limit)
    rng=random.Random(0x434f4c4c4953494f4e)
    for index in range(1200):
        low=[rng.choice([-16,-1,0,15,16])+rng.random() for _ in range(3)]
        high=[v+rng.choice([0,1e-8,2e-7,0.25,0.6,1.8,2.5]) for v in low]
        palette=rng.sample(palettes,rng.randrange(1,13))
        yield row([*low,*high],palette,bool(index%2),rng.randrange(3),rng.choice([-1,-1,-1,0,3,10,50]),rng.choice([0,1,3,100000]),rng.randrange(-2147483648,2147483648))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-block-collision-scan-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockCollisionScanOracle.java'
    helpers=[oracle.with_name('VanillaFaceSupportOracle.java')]
    sources=[oracle,*helpers,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/block_collision_scan_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBlockCollisionScanOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'block_collision_scan_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} block-collision-scan output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BlockCollisions cursor and selection algorithm over controlled block metadata, chunk providers and context shapes. Compares ordered chunk/state/suffocation/shape calls, missing-chunk retries, cached chunk reuse, face/edge/corner filtering, canonical full-block versus equal noncanonical shape behavior, exact emitted shape data, early consumer termination and injected provider failures. Finite bounded scans include thin/zero-size boxes, chunk crossings and wrapped integer positions. Registry-specific metadata/shape dispatch, live chunk loading and entity movement are separate integration work; unsupported cursor volumes and caller budgets use typed Rust errors.'}
    lab.write_json(report_path,report);print(f'Scoped block collision scan parity passed: {report_path}')


if __name__=='__main__':main()
