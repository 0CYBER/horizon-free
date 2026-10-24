#!/usr/bin/env python3
"""Compare inner living/player damage application and side-effect ordering with vanilla."""
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
    def state(damage=10,absorption=5,health=20,maximum=20,attacker=2,invulnerable=False):return dict(health=bits(health),absorption=bits(absorption),exhaustion=bits(0),max_absorption=double(maximum),armor=bits(damage),magic=bits(damage),food_exhaustion=bits(.1),attacker=attacker,invulnerable=invulnerable)
    def row(player,initial,**kw):
        f=dict(player=player,initial=initial,after=state(2,3,11,2,2),damage=bits(17),fail=-1,mutate_at=-1);f.update(kw);return f
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.049999997,.05,.15,1.,5.,10.,20.,1e30,3.4028233e37,3.4028235e37,float('inf'),float('nan')]
    for player in [False,True]:
        for damage in values:
            for absorption in values:
                for maximum in [-1.,-0.,0.,5.,20.,float('nan')]:yield row(player,state(damage,absorption,20,maximum))
        for attacker in range(3):
            for invulnerable in [False,True]:
                for fail in range(-1,21):
                    for mutation in range(-1,21):yield row(player,state(attacker=attacker,invulnerable=invulnerable),fail=fail,mutate_at=mutation)
    rng=random.Random(0x44414d414745)
    for _ in range(5000):
        f=row(bool(rng.randrange(2)),state(rng.choice(values),rng.choice(values),rng.choice(values),rng.choice(values),rng.randrange(3)),fail=rng.choice([-1,-1,0,3,7,11,15]),mutate_at=rng.choice([-1,-1,0,4,8,12]))
        if rng.randrange(3)==0:
            for key in ['armor','magic','health','absorption','food_exhaustion']:f['initial'][key]=f'{rng.getrandbits(32):08x}'
        yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-damage-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingDamageOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_damage_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingDamageOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_damage_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living damage output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity and Player actuallyHurt with observable phase/storage/stat/combat/event hooks; real final absorption clamping. Invulnerability, armor/magic dispatch, two absorption reads, absorbed statistics and Java rounding, player exhaustion/taken statistics, base-living second absorption write, health and damage-event ordering, mutation/failure retention. Armor/magic results, storage, combat/stat sinks and exhaustion delivery are controlled; outer hurt gates, full damage binding and live gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living damage parity passed: {report_path}')


if __name__=='__main__':main()
