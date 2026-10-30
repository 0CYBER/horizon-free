#!/usr/bin/env python3
"""Compare registered fluid current vectors and neighbor queries with vanilla."""
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
    water=next(s for s in states if s['id']=='minecraft:water' and s['properties']['falling']=='false')
    def cell(fluid=empty,blocks=False,ice=False,faces=0):return dict(fluid=fluid,blocks_flow=blocks,ice=ice,faces=faces)
    def row(root,entries=None,**kw):
        f=dict(mode='flow',root=root,states=entries or [cell()],default=0,cells={},position=[0,0,0],fail=-1,direction=2);f.update(kw);return f
    neighbors=[(0,0,-1),(1,0,0),(0,0,1),(-1,0,0)]
    for root in states:
        for neighbor in states:
            for blocks in [False,True]:
                yield row(root,[cell(neighbor,blocks),cell(empty,False),cell(water,False)],cells={'0 0 -1':1,'0 -1 -1':2})
    for root in states:
        for index,p in enumerate(neighbors):
            for y in [0,1]:
                for mask in range(64):
                    for ice in [False,True]:
                        key=f'{p[0]} {y} {p[2]}'
                        yield row(root,[cell(),cell(empty,False,ice,mask)],cells={key:1})
    for root in states[1:]:
        for neighbor in [empty,water,states[-1]]:
            for direction in range(6):
                for ice in [False,True]:
                    for mask in [0,63,1<<direction]:yield row(root,[cell(neighbor,False,ice,mask)],mode='face',direction=direction)
    rng=random.Random(0x464c4f57)
    for _ in range(6000):
        p=rng.choice([(0,0,0),(17,-64,-33),(2147483647,-2147483648,-2147483648)])
        def wrap(v):return (v+2**31)%2**32-2**31
        cells={' '.join(str(wrap(a+b)) for a,b in zip(p,(dx,y,dz))):rng.randrange(6) for dx,_,dz in neighbors for y in [-1,0,1]}
        yield row(rng.choice(states),[cell(rng.choice(states),bool(rng.randrange(2)),bool(rng.randrange(2)),rng.randrange(64)) for _ in range(6)],cells=cells,position=p,fail=rng.choice([-1,-1,*range(35)]))
    for root in states:
        for fail in range(-1,35):yield row(root,[cell(),cell(water)],cells={'0 -1 -1':1},fail=fail)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-fluid-flow-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFluidFlowOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/fluid_flow_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaFluidFlowOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'fluid-states.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'fluid-states.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'fluid_flow_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} fluid-flow output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual registered FluidState.getFlow / FlowingFluid.getFlow and solid-face helper over every registered fluid state. Ordered horizontal and downward neighbor reads, fluid families, BLOCKS_FLUID_FLOW tag, IceBlock guard, falling-wall deflection, f32 height differences, vector normalization, wrapping coordinates and injected failures. Block flags and face support are controlled; registry/world support binding, spreading and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped fluid flow parity passed: {report_path}')


if __name__=='__main__':main()
