#!/usr/bin/env python3
"""Compare world ray clipping, interaction faces and fall-reset selection with vanilla."""
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
    import itertools
    def bits(v):return struct.pack('>d',float(v)).hex()
    def shape(boxes):return [[bits(v) for v in b] for b in boxes]
    def state(**kw):
        f=dict(collider=[],outline=[],visual=[],interaction=[],fluid_shape=[],reset=False,gateway=False,end=False,nether=False,source=False,empty=True,water=False);f.update(kw);return f
    full=shape([[0,0,0,1,1,1]])
    def row(s=None,**kw):
        f=dict(states=[state(),s or state(collider=full,outline=full,visual=full)],cells={'0 0 0':1},block_mode=0,fluid_mode=0,entity=False,player=False,server=False,delay=80,fail=-1,**{'from':list(map(bits,[-2,.5,.5])),'to':list(map(bits,[2,.5,.5]))});f.update(kw);return f
    shapes=[[],full,shape([[0,0,0,1,.5,1]]),shape([[.5,0,0,1,1,1]]),shape([[0,.25,0,1,.75,1]]),shape([[0,0,0,.25,1,1],[.75,0,0,1,1,1]]),shape([[-.5,-.5,-.5,1.5,1.5,1.5]]),shape([[0,0,0,0,0,0]])]
    rays=[([-2,.5,.5],[2,.5,.5]),([.5,2,.5],[.5,-2,.5]),([.5,.5,.5],[2,2,2]),([-1,-1,-1],[2,2,2]),([0,0,0],[0,0,0]),([-0.,0,0],[0,0,0]),([1,.5,.5],[0,.5,.5]),([.5,.5,.5],[.5+1e-8,.5,.5]),([.5,.5,.5],[.5+.001,.5,.5]),([2,2,2],[3,3,3])]
    for block,fluid,interaction,mode,fluid_mode in itertools.product(shapes,shapes,shapes[:5],range(3),range(4)):
        ray=rays[(mode+fluid_mode)%len(rays)]
        yield row(state(collider=block,outline=block,visual=block,interaction=interaction,fluid_shape=fluid,source=True,empty=False,water=True),block_mode=mode,fluid_mode=fluid_mode,**{'from':list(map(bits,ray[0])),'to':list(map(bits,ray[1]))})
    for flags,entity,player,server,delay,fluid_mode in itertools.product(range(16),[False,True],[False,True],[False,True],[-1,0,1,80],range(4)):
        yield row(state(reset=bool(flags&1),gateway=bool(flags&2),end=bool(flags&4),nether=bool(flags&8),interaction=full,fluid_shape=full,source=True,empty=False,water=True),block_mode=3,fluid_mode=fluid_mode,entity=entity,player=player,server=server,delay=delay)
    for ray,mask,mode in itertools.product(rays,range(8),range(4)):
        yield row(state(collider=full,outline=shapes[2],visual=shapes[3],interaction=shapes[4],fluid_shape=full,source=bool(mask&1),empty=bool(mask&2),water=bool(mask&4)),fluid_mode=mode,**{'from':list(map(bits,ray[0])),'to':list(map(bits,ray[1]))})
    for mode,fluid_mode,fail in itertools.product(range(4),range(4),range(-1,42)):
        yield row(state(collider=full,outline=full,visual=full,interaction=full,fluid_shape=full,nether=True,source=True,empty=False,water=True),block_mode=mode,fluid_mode=fluid_mode,fail=fail,entity=True,player=True,server=True,delay=0)
    rng=random.Random(0x524159434c4950)
    for _ in range(7000):
        sh=[rng.choice(shapes) for _ in range(5)];p=[rng.choice([-2,-1,-0.,0.,.25,.5,1.,2.,3.]) for _ in range(3)];q=[rng.choice([-2,-1,-0.,0.,.25,.5,1.,2.,3.]) for _ in range(3)]
        yield row(state(collider=sh[0],outline=sh[1],visual=sh[2],interaction=sh[3],fluid_shape=sh[4],reset=bool(rng.randrange(2)),gateway=bool(rng.randrange(2)),end=bool(rng.randrange(2)),nether=bool(rng.randrange(2)),source=bool(rng.randrange(2)),empty=bool(rng.randrange(2)),water=bool(rng.randrange(2))),block_mode=rng.randrange(4),fluid_mode=rng.randrange(4),entity=bool(rng.randrange(2)),player=bool(rng.randrange(2)),server=bool(rng.randrange(2)),delay=rng.choice([-1,0,1,80]),cells={rng.choice(['0 0 0','-1 -1 -1','1 1 1']):1 for _ in range(3)},fail=rng.randrange(-1,30),**{'from':list(map(bits,p)),'to':list(map(bits,q))})


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-world-ray-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaWorldRayOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/world_ray_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaWorldRayOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'world_ray_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} world-ray output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BlockGetter.clip, clipWithInteractionOverride and native ClipContext block/fluid selection. Covers collision/outline/visual/fall-reset modes, none/source/any/water fluid modes, nearest-hit ties, face-only interaction overrides, inside hits, misses, zero/signed-zero/tiny rays, ordered block/fluid reads and provider failures. Fall-reset tests include block tag, entity type identity, end gateways/portals and server nether-portal delay with lazy rule queries. Shapes and state predicates are controlled providers; registered outline/visual/interaction shapes, live world binding and gameplay integration remain separate work. Nonfinite rays and exhausted visit budgets return explicit Rust errors rather than fabricated misses.'}
    lab.write_json(report_path,report);print(f'Scoped world ray parity passed: {report_path}')


if __name__=='__main__':main()
