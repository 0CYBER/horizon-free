#!/usr/bin/env python3
"""Compare movement distance, step, swim and flap event dispatch with vanilla."""
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
    def single(v):return struct.pack('>f',float(v)).hex()
    def state(mask):return dict(air=bool(mask&1),climbable=bool(mask&2),powder=bool(mask&4))
    def row(**kw):
        f=dict(block=False,states=[state(0),state(0)],emission=3,flap_emission=3,movement=list(map(bits,[1.,.2,1.])),position=list(map(bits,[17.3,-63.,-32.3])),on_pos=[17,-64,-33],legacy=[17,-64,-33],ground=True,crouch=False,rails=False,swimming=False,water=False,flapping=False,sounds=True,events=True,move=single(1.),fly=single(0.),next=single(1.),mutate=False,mutated_move=single(12.5),mutated_fly=single(-3.),mutated_next=single(9.),fail=-1)
        f.update(kw);return f
    for old,new,flags,emission,same in itertools.product(range(8),range(8),range(64),range(4),[False,True]):
        yield row(states=[state(old),state(new)],emission=emission,flap_emission=emission,legacy=[17,-64,-33] if same else [17,-65,-33],ground=bool(flags&1),crouch=bool(flags&2),rails=bool(flags&4),swimming=bool(flags&8),water=bool(flags&16),flapping=bool(flags&32))
    for st,flags,y,sounds,events in itertools.product(range(8),range(16),[-0.,.1,math.nan],[False,True],[False,True]):
        yield row(block=True,states=[state(st),state(0)],movement=list(map(bits,[1,y,1])),ground=bool(flags&1),crouch=bool(flags&2),rails=bool(flags&4),swimming=bool(flags&8),sounds=sounds,events=events)
    values=[-math.inf,-1e30,-1.,-0.,0.,1e-38,1e-7,.99999994,1.,1.00000012,2147483520.,2147483648.,1e30,math.inf,math.nan]
    for move,next_value,y,st in itertools.product(values,values,[-0.,.1,math.nan],range(4)):
        yield row(move=single(move),next=single(next_value),fly=single(move),movement=list(map(bits,[1.,y,1.])),states=[state(0),state(st)])
    # Failures before and after state writes and sound/event callbacks.
    for old,new,flags,fail,mutate in itertools.product([0,1,2],[0,1,2],range(8),range(-1,27),[False,True]):
        yield row(states=[state(old),state(new)],ground=bool(flags&1),water=bool(flags&2),flapping=bool(flags&4),legacy=[17,-65,-33],fail=fail,mutate=mutate)
    rng=random.Random(0x454d495353494f4e)
    for _ in range(12000):
        yield row(block=bool(rng.randrange(2)),states=[state(rng.randrange(8)),state(rng.randrange(8))],emission=rng.randrange(4),flap_emission=rng.randrange(4),movement=[bits(rng.choice(values+[1e200,1e-200])) for _ in range(3)],ground=bool(rng.randrange(2)),crouch=bool(rng.randrange(2)),rails=bool(rng.randrange(2)),swimming=bool(rng.randrange(2)),water=bool(rng.randrange(2)),flapping=bool(rng.randrange(2)),sounds=bool(rng.randrange(2)),events=bool(rng.randrange(2)),move=single(rng.choice(values)),fly=single(rng.choice(values)),next=single(rng.choice(values)),mutate=bool(rng.randrange(2)),mutated_move=single(rng.choice(values)),mutated_fly=single(rng.choice(values)),mutated_next=single(rng.choice(values)),fail=rng.randrange(-1,23))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-movement-emission-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMovementEmissionOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/movement_emission_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaMovementEmissionOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'movement_emission_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} movement-emission output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity movement emission, private block vibration/step dispatch, flapping dispatch and native next-step arithmetic. Compares distance accumulation, climbable/powder predicates, legacy/current support ordering, ground/crouch/rail/swimming guards, sound/event emissions, float boundaries, NaN/infinity, callback mutations and partial state on failures. Block predicates, world reads and sound callbacks are controlled; crystal sound predicate is false in this scope. Registered sound selection, audio playback, full entity movement and gameplay remain separate work.'}
    lab.write_json(report_path,report);print(f'Scoped movement emission parity passed: {report_path}')


if __name__=='__main__':main()
