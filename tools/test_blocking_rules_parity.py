#!/usr/bin/env python3
"""Compare native blocking reductions, timers and item-damage arithmetic."""
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
    def bits(v):return struct.pack('>f',v).hex()
    def double(v):return struct.pack('>d',v).hex()
    def reduction(angle=90,base=0,factor=1,set=0):return dict(angle=bits(angle),base=bits(base),factor=bits(factor),set=set)
    def row(damage=8,angle=0,reductions=None,delay=.25,scale=1,seconds=1,threshold=3,base=1,factor=1,**kw):
        f=dict(damage=bits(damage),angle=double(angle),reductions=reductions if reductions is not None else [reduction()],delay=bits(delay),disable_scale=bits(scale),disable_seconds=bits(seconds),item=dict(threshold=bits(threshold),base=bits(base),factor=bits(factor)),type=0,after_type=1,fail=-1,mutate_at=-1);f.update(kw);return f
    values=[-float('inf'),-1e30,-1,-.5,-0.,0.,1e-40,.1,.5,1,5,20,1e30,float('inf'),float('nan')]
    for damage in values:
        for base in values:
            for factor in values:
                yield row(damage,base=base,factor=factor,threshold=base,delay=damage,scale=base,seconds=factor,reductions=[reduction(base=base,factor=factor),reduction(angle=float('nan'),base=factor,factor=base,set=1)])
    for degrees in [-float('inf'),-360,-90,-0.,0.,1e-40,.1,45,90,180,360,float('inf'),float('nan')]:
        c=struct.unpack('>f',struct.pack('>f',.017453292))[0]
        radians=struct.unpack('>f',struct.pack('>f',c*degrees))[0]
        for angle in [radians,math.nextafter(radians,-float('inf')),math.nextafter(radians,float('inf')),-float('inf'),float('inf'),float('nan')]:
            for set in range(5):
                for t in range(2):yield row(angle=angle,reductions=[reduction(angle=degrees,set=set)],type=t)
    for delay in [i/40 for i in range(-100,101)]+[53687088.,107374176.,107374184.,float('nan')]:
        yield row(delay=delay,scale=1,seconds=delay)
    rng=random.Random(0x424c4f434b52554c)
    def random_float():return struct.unpack('>f',struct.pack('>I',rng.getrandbits(32)))[0]
    for _ in range(10000):
        v=lambda:rng.choice(values) if rng.randrange(2) else random_float()
        yield row(damage=v(),angle=rng.choice([float('nan'),-float('inf'),float('inf'),rng.uniform(-10,10)]),reductions=[reduction(angle=v(),base=v(),factor=v(),set=rng.randrange(5)) for _ in range(rng.randrange(9))],delay=v(),scale=v(),seconds=v(),threshold=v(),base=v(),factor=v(),type=rng.randrange(2),after_type=rng.randrange(2),fail=rng.choice([-1,-1,*range(8)]),mutate_at=rng.randrange(-1,8))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-blocking-rules-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockingRulesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/blocking_rules_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBlockingRulesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'blocking_rules_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} blocking rules output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BlocksAttacks damage reduction records and ordered sums, native direct HolderSet membership through virtual source typeHolder queries, delay/disable tick rounding and ItemDamageFunction thresholds/flooring. Covers NaN/infinity/signed zero, exact angle boundaries, randomized float bits, membership mutations and query failures. Direction/acos calculation, component codecs, item durability, sounds and full blocking dispatch remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped blocking rules parity passed: {report_path}')


if __name__=='__main__':main()
