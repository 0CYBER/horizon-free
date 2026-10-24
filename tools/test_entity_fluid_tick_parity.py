#!/usr/bin/env python3
"""Compare Entity fluid tick composition, entry effects and current forces with vanilla."""
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


try:
    from . import test_entity_fluid_parity as immersion
except ImportError:
    import test_entity_fluid_parity as immersion

def corpus():
    import copy
    def bits(v):return struct.pack('>d',float(v)).hex()
    rng=random.Random(0x464c5549445449434b)
    def convert(f,**kw):
        f=copy.deepcopy(f);f['tick_initial']=dict(touch=kw.get('touch',False),first=kw.get('first',False),fall=bits(8))
        for op in f['ops']:
            if op['mode']=='update':op['mode']='tick'
            op.update(pushed=kw.get('pushed',[True,True]),fast_lava=kw.get('fast',False),after_touch=kw.get('after_touch',True),after_first=kw.get('after_first',True),after_fall=bits(-3))
        return f
    cases=list(immersion.corpus())
    for f in cases:
        yield convert(f,touch=bool(rng.randrange(2)),first=bool(rng.randrange(2)),pushed=rng.choice([[True],[False],[False,True],[True,False]]),fast=bool(rng.randrange(2)),after_touch=bool(rng.randrange(2)),after_first=bool(rng.randrange(2)))
    seed=copy.deepcopy(cases[0]);seed['ops']=seed['ops'][:1]
    op=seed['ops'][0];op['bounds']=[bits(v) for v in [0,0,0,1,1,2]];op['eye']=bits(.5)
    op['cells']={'0 0 0':dict(kind=2,height=struct.pack('>f',.8).hex(),flow=[bits(v) for v in [1,0,0]]),'0 0 1':dict(kind=4,height=struct.pack('>f',.8).hex(),flow=[bits(v) for v in [0,0,1]])}
    for flags in range(64):
        f=convert(seed,touch=bool(flags&1),first=bool(flags&2),pushed=[bool(flags&4),bool(flags&8)],fast=bool(flags&16),after_touch=bool(flags&32),after_first=False)
        for fail in range(-1,29):
            for mutate in [-1,0,18,21,24]:
                g=copy.deepcopy(f);g['ops'][0]['fail']=fail;g['ops'][0]['mutate_at']=mutate;yield g
    for f in cases[:100]:
        g=convert(f);g['ops']*=3;yield g


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-entity-fluid-tick-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityFluidOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_entity_fluid_parity.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/entity_fluid_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEntityFluidOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'entity_fluid_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} entity-fluid-tick output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity.updateFluidInteraction composing EntityFluidInteraction update and current application. Compares both push predicate calls, water-entry fall reset and splash dispatch, retained touching-water state, first tick, water/lava current coefficients and FAST_LAVA dimension attribute selection. Includes ordered/reloaded tags, controlled cell heights/flows, player/nonplayer classes, mutations and failures before/after partial writes. Splash audio/particles and full live world/entity gameplay binding remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped entity fluid tick parity passed: {report_path}')


if __name__=='__main__':main()
