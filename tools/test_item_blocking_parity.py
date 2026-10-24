#!/usr/bin/env python3
"""Compare actual living item-blocking dispatch with native components."""
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
    def single(v):return struct.pack('>f',v).hex()
    def double(v):return struct.pack('>d',v).hex()
    def vec(v):return [double(x) for x in v]
    def reduction(angle=90,base=0,factor=1,set=0):return dict(angle=single(angle),base=single(base),factor=single(factor),set=set)
    def component(present=True,bypass=0,reductions=None,threshold=3,base=1,factor=1):return dict(present=present,bypass=bypass,reductions=reductions if reductions is not None else [reduction()],item_damage=dict(threshold=single(threshold),base=single(base),factor=single(factor)))
    def state(**kw):
        s=dict(item=0,direct=1,source=vec([0,0,1]),position=vec([0,0,0]),yaw=single(0),pierce=0,projectile=False,type=0,level=0,hand=0,client=False);s.update(kw);return s
    def row(damage=8,initial=None,after=None,components=None,**kw):
        f=dict(damage=single(damage),initial=initial or state(),after=after or state(),components=components or [component(),component(reductions=[reduction(angle=180,factor=.5,set=2)])],player=False,fail=-1,mutate_at=-1);f.update(kw);return f
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,5,20,1e30,float('inf'),float('nan')]
    for damage in values:
        for direct in range(4):
            for pierce in [-128,-1,0,1,127]:
                for player in [False,True]:
                    for bypass in range(5):yield row(damage,initial=state(direct=direct,pierce=pierce),components=[component(bypass=bypass),component()],player=player)
    for fail in range(-1,26):
        for mutate in range(-1,26):
            for player in [False,True]:yield row(initial=state(direct=2,pierce=0),after=state(item=1,direct=1,source=None,position=vec([.5,0,0]),yaw=single(45),type=1,level=1,hand=1,client=True),components=[component(bypass=2,reductions=[reduction(set=1),reduction(angle=180,factor=.5)]),component()],player=player,fail=fail,mutate_at=mutate)
    rng=random.Random(0x4954454d424c4f43)
    def random_state():return state(item=rng.randrange(-1,2),direct=rng.randrange(4),source=None if rng.randrange(4)==0 else vec([rng.choice(values) for _ in range(3)]),position=vec([rng.choice(values) for _ in range(3)]),yaw=single(rng.choice(values+[45,90,180,360])),pierce=rng.randrange(-128,128),projectile=bool(rng.randrange(2)),type=rng.randrange(2),level=rng.randrange(2),hand=rng.randrange(2),client=bool(rng.randrange(2)))
    for _ in range(8000):yield row(rng.choice(values),initial=random_state(),after=random_state(),components=[component(present=rng.randrange(4)!=0,bypass=rng.randrange(5),reductions=[reduction(angle=rng.choice(values+[45,90,180,360]),base=rng.choice(values),factor=rng.choice(values),set=rng.randrange(5)) for _ in range(rng.randrange(6))],threshold=rng.choice(values),base=rng.choice(values),factor=rng.choice(values)) for _ in range(2)],player=bool(rng.randrange(2)),fail=rng.choice([-1,-1,*range(26)]),mutate_at=rng.randrange(-1,26))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-blocking-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemBlockingOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_blocking_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemBlockingOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_blocking_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item blocking output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity.applyItemBlocking on base and ServerPlayer receivers, using native BlocksAttacks reductions/holder sets and actual stat/durability dispatch on undamageable stacks. Tests bypass/piercing, source direction, component/item capture across mutations, client/stat guards, repeated direct-entity lookup, melee response and callback failures. Actual damageable-item mutation, blocking-item eligibility/timing, component codecs and full combat/world integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item blocking parity passed: {report_path}')


if __name__=='__main__':main()
