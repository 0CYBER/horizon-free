#!/usr/bin/env python3
"""Compare entity obstruction queries and shape intersection with pinned vanilla."""
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
    def entity(bounds,building=True,removed=False,spectator=False,parent=-1):
        return {'box':box(bounds),'building':building,'removed':removed,'spectator':spectator,'parent':parent}
    unit=[0,0,0,1,1,1]
    shapes=[[],[unit],[[0,0,0,1,0.5,1]],[[0,0,0,1,0.25,1],[0,0.75,0,1,1,1]],[[0,0,0,1e-8,1,1]],[[1,1,1,0,0,0]],[[2147483647,0,-2147483648,2147483648,1,-2147483647]]]
    boxes=[unit,[0.2,0.4,0.2,0.8,0.6,0.8],[1,0,0,2,1,1],[0.5,0.5,0.5,0.5,0.75,0.75],[-1e-8,0,0,1e-8,1,1],[1-1e-8,0,0,2,1,1],[1-2e-7,0,0,2,1,1],[2,2,2,3,3,3],[2147483647,0,-2147483648,2147483648,1,-2147483647]]
    variants=[[]]
    for bounds in boxes:
        for flags in range(8):variants.append([entity(bounds,bool(flags&1),bool(flags&2),bool(flags&4))])
    for parent in [-1,0]:
        variants.append([entity(unit),entity(unit,parent=parent),entity([0.2,0.4,0.2,0.8,0.6,0.8],parent=1)])
        variants.append([entity(unit,False),entity(unit,parent=parent),entity(unit,removed=True,parent=1)])
    for shape in shapes:
        for entities in variants:
            for exclude in range(-1,len(entities)):
                for fail in [False,True]:yield {'shape':[box(v) for v in shape],'entities':entities,'exclude':exclude,'fail':fail}
    rng=random.Random(0x484f52495a4f4e)
    extents=[-1.0,0.0,1e-8,1e-7,2e-7,0.125,0.5,1.0,2.0]
    def random_box():
        low=[rng.choice([-2.0,-1e-8,0.0,0.5,1.0-1e-8,1.0,2.0]) for _ in range(3)]
        return [*low,*[v+rng.choice(extents) for v in low]]
    for index in range(3000):
        shape=[box(random_box()) for _ in range(rng.randrange(1,5))]
        entities=[entity(random_box(),rng.choice([True,True,False]),rng.randrange(4)==0,rng.randrange(4)==0,rng.randrange(-1,i) if i else -1) for i in range(rng.randrange(1,13))]
        yield {'shape':shape,'entities':entities,'exclude':rng.randrange(-1,len(entities)),'fail':index%17==0}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-entity-obstruction-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityObstructionOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/entity_obstruction_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEntityObstructionOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'entity_obstruction_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} entity-obstruction output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual EntityGetter.isUnobstructed, its default non-spectator query, Entity removal/root-vehicle methods and voxel intersection execute over privately allocated entity snapshots. Empty shapes, excluded identities, spectators, removed/non-building entities, same-root vehicle exclusions, ordered broad-phase queries and selected candidates, thin boxes, boundary contacts, composite shapes and query failures are compared. Entity construction, lifecycle, spatial indexing, vehicle maintenance and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped entity-obstruction parity passed: {report_path}')


if __name__=='__main__':main()
