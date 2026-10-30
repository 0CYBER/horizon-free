#!/usr/bin/env python3
"""Compare cached registered fluid shapes and lazy height queries with vanilla."""
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


def corpus(states):
    empty=next(s for s in states if s['id']=='minecraft:empty')
    def op(state,above,p=(17,-64,-33),fail=-1):return dict(state=state,above=above,position=p,fail=fail)
    for state in states:
        for above in states:
            for fail in [-1,0,1]:
                yield dict(ops=[op(state,above,fail=fail),op(state,empty,p=(1,2,3),fail=0),op(state,above,p=(-1,-2,-3)),op(state,empty)])
    for above in states:
        for fail in [-1,0]:
            yield dict(ops=[*[op(state,above,p=(2147483647,2147483647,-2147483648),fail=fail) for state in states],*[op(state,empty,p=(-2147483648,-2147483648,2147483647)) for state in reversed(states)],*[op(state,above,fail=0) for state in states]])
    rng=random.Random(0x464c554944534850)
    for _ in range(1000):yield dict(ops=[op(rng.choice(states),rng.choice(states),p=rng.choice([(17,-64,-33),(0,0,0),(2147483647,2147483647,-2147483648)]),fail=rng.choice([-1,-1,0,1])) for _ in range(rng.randrange(1,100))])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-fluid-shape-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFluidShapeOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/fluid_shape_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFluidShapeOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'fluid-states.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'fluid-states.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'fluid_shape_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} fluid-shape output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual registered FluidState.getShape and FlowingFluid first-query shape caches over every native fluid state. Compares ordered above-fluid queries, cached-state counts, shape geometry, canonical empty/full identities and aliases across repeated states. Sequences change above fluids, positions, failure points and query order; failed height reads do not populate a cache. Caches are reset between sequences and shared across world views within each sequence. Fluid propagation, entity immersion, registered world ray binding and full gameplay remain separate work.'}
    lab.write_json(report_path,report);print(f'Scoped fluid shape parity passed: {report_path}')


if __name__=='__main__':main()
