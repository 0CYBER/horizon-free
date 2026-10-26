#!/usr/bin/env python3
"""Compare EntityFluidInteraction immersion and current accumulation with vanilla."""
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
    def bits(v):return struct.pack('>d',float(v)).hex()
    def single(v):return struct.pack('>f',float(v)).hex()
    def cell(kind=2,height=8/9,flow=(1,0,.25)):
        return dict(kind=kind,height=single(height),flow=[bits(v) for v in flow])
    def op(**kw):
        v=dict(mode='update',bounds=[.1,.1,.1,.9,.9,.9],base_y=0,eye=.8,block=[0,0,0],ignore=False,min_section=-1,wet=[False,True,False],missing=[],sturdy=False,cells={},default=cell(),tags=[6,24,30,0],velocity=[0,0,0],after_velocity=[1,2,3],after_block=[1,0,1],mutate_at=-1,fail=-1,tag=0,speed=.014)
        v.update(kw)
        for k in ['base_y','eye','speed']:v[k]=bits(v[k])
        for k in ['bounds','velocity','after_velocity']:
            if v[k] is not None:v[k]=[bits(n) for n in v[k]]
        return v
    def row(ops,player=False,order=(0,1,2)):return dict(ops=ops,player=player,order=order)
    # Dropped items: the 0.25-wide box is deflated by Entity before scanning.
    # Cover surface grazing, buoyancy depth, cell boundaries and repeated pushes.
    for x in [.5, 1.0, 15.99]:
        for y in [0, .1, .7, 8/9-.1, 8/9-.001, 8/9, 1]:
            for flow in [(0,0,0),(1,0,0),(-1,0,1),(0,-1,0)]:
                update=op(bounds=[x-.124,y+.001,.376,x+.124,y+.249,.624],
                          base_y=y,eye=y+.2125,block=[int(x),int(y),0],
                          default=cell(flow=flow))
                yield row([update,op(mode='apply'),update,op(mode='apply')])
    values=[-float('inf'),-1,-.0,0,.1,.39999998,.4,.40000004,8/9,1,2,float('inf'),float('nan')]
    for kind in range(5):
        for height in values:
            for eye in values:
                for sturdy in [False,True]:
                    yield row([op(default=cell(kind,height),eye=eye,sturdy=sturdy),op(mode='apply'),op(bounds=None),op(bounds=None)])
    for player in [False,True]:
        for flow in [(0,0,0),(.0031622776,0,0),(.0031622777,0,0),(1,0,.25),(-1,0,1),(1,float('nan'),2),(float('inf'),0,0)]:
            for speed in values+[.003,.0045,.004500000000000001]:
                for velocity in [(0,0,0),(.003,0,0),(.0029999999999,0,0),(0,0,.003),(float('nan'),0,0)]:
                    yield row([op(bounds=[0,0,0,2,1,1],default=cell(flow=flow)),op(mode='apply',speed=speed,velocity=velocity)],player)
    cells={'0 0 0':cell(1,.2,(1,0,0)),'0 0 1':cell(2,.3,(0,1,0)),'0 1 0':cell(3,.4,(0,0,1)),'0 1 1':cell(4,.5,(-1,0,0)),'1 0 0':cell(1,.1,(0,-1,0))}
    for order in [[],[0],[0,1,2],[2,0,1],[0,0,1],[3,1,0]]:
        for ignore in [False,True]:
            for fail in range(-1,70):
                for mutate in [-1,9]:
                    yield row([op(bounds=[0,0,0,2,2,2],cells=cells,default=cell(0),ignore=ignore,fail=fail,mutate_at=mutate),*[op(mode='apply',tag=t,mutate_at=0) for t in range(4)],op(default=cell(3),tags=[24,6,30,0]),op(default=cell(0)),op(default=cell(0))],order=order)
    for bounds in [[0,0,0,0,0,0],[-17,-17,-17,-16,-16,-16],[-.1,15.9,-.1,.1,16.1,.1],[15,0,15,17,1,17],[17,0,17,15,1,15],[0,32,0,1,33,1]]:
        for wet in [[False,False,False],[True,False,False],[False,True,False],[False,False,True],[True,True,True]]:
            for missing in [[],['0 0'],['1 1'],['-1 -1']]:yield row([op(bounds=bounds,wet=wet,missing=missing)])
    for lo,hi in [(-2147483649.,-2147483648.),(-1e30,-1e30),(2147483648.,2147483649.),(1e30,1e30)]:
        for axis in range(3):
            bounds=[0.,0.,0.,1.,1.,1.];bounds[axis]=lo;bounds[axis+3]=hi
            yield row([op(bounds=bounds,fail=2)])
    rng=random.Random(0x454e54464c554944)
    for _ in range(3000):
        ops=[]
        for i in range(rng.randrange(1,7)):
            ops.append(op(mode=rng.choice(['update','update','apply']),bounds=rng.choice([None,[0,0,0,2,2,2],[.1,.1,.1,.9,.9,.9],[-1,-1,-1,1,1,1]]),cells=cells,default=cell(rng.randrange(5),rng.choice(values),tuple(rng.choice(values) for _ in range(3))),base_y=rng.choice(values),eye=rng.choice(values),sturdy=bool(rng.randrange(2)),ignore=bool(rng.randrange(2)),tags=[rng.randrange(32) for _ in range(4)],fail=rng.choice([-1,-1,*range(25)]),mutate_at=rng.choice([-1,-1,*range(15)]),speed=rng.choice(values),tag=rng.randrange(4)))
        yield row(ops,bool(rng.randrange(2)),rng.sample(range(4),rng.randrange(5)))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-entity-fluid-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityFluidOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/entity_fluid_fixture.rs']
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
        print(f'{side}: {matched:,} entity-fluid output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual EntityFluidInteraction update, tracker pruning, body/eye depth, loaded-chunk preflight, overlapping/reloaded tags, ordered current accumulation and native Entity.addDeltaMovement. Controlled fluid height/flow and camera-support cells, real ProtoChunk section fluid flags, player versus nonplayer classes, callback mutations and injected failures. Live fluid flow/propagation and full entity/world integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped entity fluid parity passed: {report_path}')


if __name__=='__main__':main()
