#!/usr/bin/env python3
"""Compare native blocking-item damage bound to actual item-stack durability."""
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
    floats = [-float('inf'), -1e30, -1, -0., 0., 1e-40, .5, 1, 5, 20, 1e30, float('inf'), float('nan')]
    def state(count=1, maximum=20, damage=0, unbreakable=False): return dict(count=count, max=maximum, damage=damage, unbreakable=unbreakable)
    def rule(threshold=3, base=1, factor=1):return dict(threshold=bits(threshold),base=bits(base),factor=bits(factor))
    def row(damage=20, **kw):
        f=dict(damage=bits(damage),rule=rule(),air=False,kind=2,hand=0,infinite=False,after_infinite=False,client=False,after_client=False,server=True,after_server=True,initial=state(),after=state(count=2,maximum=5,damage=4),fail=-1,mutate_at=-1);f.update(kw);return f
    for damage in floats:
        for base in floats:
            for factor in floats:
                for kind in range(3):yield row(damage,rule=rule(threshold=base,base=base,factor=factor),kind=kind)
    for kind in range(3):
        for mutate in range(-1,7):
            for fail in range(-1,7):
                for count in [0,1,2]:
                    for maximum in [None,0,5,20]:yield row(kind=kind,fail=fail,mutate_at=mutate,hand=1,after=state(count=count,maximum=maximum,damage=4))
    rng=random.Random(0x424c4f434b445552)
    edges=[-2147483648,-1,0,1,2,19,20,21,16777217,2147483647]
    def integer():return rng.choice(edges) if rng.randrange(2) else rng.randrange(-2147483648,2147483648)
    def random_state():return state(rng.choice([-1,0,1,2,64]),rng.choice([None,integer()]),rng.choice([None,integer()]),bool(rng.randrange(2)))
    def floating():return bits(rng.choice(floats)) if rng.randrange(2) else f'{rng.getrandbits(32):08x}'
    for _ in range(12000):
        flags={name:bool(rng.randrange(2)) for name in ['infinite','after_infinite','client','after_client','server','after_server']}
        yield row(damage=0,**flags,rule={name:floating() for name in ['threshold','base','factor']},kind=rng.randrange(3),hand=rng.randrange(2),air=bool(rng.randrange(5)==0),initial=random_state(),after=random_state(),fail=rng.choice([-1,-1,*range(7)]),mutate_at=rng.randrange(-1,7)) | dict(damage=floating())


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-blocking-item-damage-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockingItemDamageOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/blocking_item_damage_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBlockingItemDamageOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'blocking_item_damage_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} blocking item damage output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BlocksAttacks hurtBlockingItem into native unenchanted ItemStack durability storage, on non-player, Player and ServerPlayer receivers. Native statistic item capture, client/level guards, scalar item-damage rules, creative immunity, empty-listener advancement ordering, stack mutation/shrink and equipment-break callback copy/slot are compared with Rust composition. Covers scalar float boundaries, absent/invalid durability components, callback storage mutations and failures. Complete components/codecs, enchantment effects, inventory/equipment lifecycle and full combat/world integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped blocking item damage parity passed: {report_path}')


if __name__=='__main__':main()
