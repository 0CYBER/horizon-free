#!/usr/bin/env python3
"""Compare living armor/resistance/protection dispatch and bypass rules with vanilla."""
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
    def state(armor=20,toughness=8,amplifier=0,**kw):
        s=dict(armor=double(armor),toughness=double(toughness),amplifier=amplifier,resistance=True,server=True,weapon=True,bypass_armor=False,bypass_effects=False,bypass_resistance=False,bypass_enchantments=False);s.update(kw);return s
    def row(mode,damage=10,initial=None,**kw):
        f=dict(mode=mode,damage=bits(damage),initial=initial or state(),after=state(5,3,4,resistance=False,server=False,weapon=False),player=False,attacker=2,fail=-1,mutate_at=-1);f.update(kw);return f
    values=[-float('inf'),-1e30,-20,-1.9,-1,-0.,0.,1e-40,.05,1,5,20,40,1e30,3.4028235e37,float('inf'),float('nan')]
    for armor in values+[2147483647.,2147483648.,-2147483649.]:yield row('armor-value',initial=state(armor))
    for damage in values:
        for armor in values:
            for toughness in [-8.,-1.,0.,8.,float('inf'),float('nan')]:yield row('armor',damage,state(armor,toughness))
        for amplifier in [-2**31,-429496730,-6,-1,0,1,3,4,5,2**31-1]:
            for player in [False,True]:
                for attacker in range(3):yield row('magic',damage,state(amplifier=amplifier),player=player,attacker=attacker)
    for flags in range(128):
        initial=state(resistance=flags&1!=0,server=flags&2!=0,weapon=flags&4!=0,bypass_armor=flags&8!=0,bypass_effects=flags&16!=0,bypass_resistance=flags&32!=0,bypass_enchantments=flags&64!=0)
        for mode in ['armor','magic']:yield row(mode,initial=initial)
    for mode in ['armor-value','armor','magic']:
        for player in [False,True]:
            for fail in range(-1,17):
                for mutation in range(-1,17):yield row(mode,initial=state(amplifier=1),player=player,fail=fail,mutate_at=mutation)
    rng=random.Random(0x524553495354)
    for _ in range(7000):
        initial=state(rng.choice(values),rng.choice(values),rng.randrange(-2**31,2**31),resistance=bool(rng.randrange(2)),server=bool(rng.randrange(2)),weapon=bool(rng.randrange(2)),bypass_armor=bool(rng.randrange(2)),bypass_effects=bool(rng.randrange(2)),bypass_resistance=bool(rng.randrange(2)),bypass_enchantments=bool(rng.randrange(2)))
        f=row(rng.choice(['armor-value','armor','magic']),rng.choice(values),initial,player=bool(rng.randrange(2)),attacker=rng.randrange(3),fail=rng.choice([-1,-1,0,4,8,12]),mutate_at=rng.choice([-1,-1,0,3,7,11]))
        if rng.randrange(3)==0:f['damage']=f'{rng.getrandbits(32):08x}'
        yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-damage-reduction-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDamageReductionOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/damage_reduction_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaDamageReductionOracle',mode,path],side+'-'+mode)
    java('server','export',build)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'slots.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'damage_reduction_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} damage reduction output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity armor-value and armor/magic reduction methods for base living and ServerPlayer receivers. Damage bypass tags, hurtArmor dispatch, floor/narrow attribute arithmetic, native CombatRules, resistance amplifier overflow and resisted statistics, repeated attacker lookup, native empty-equipment protection lookup, mutations/failures and numeric edges. Item enchantments, actual durability/effects/tag registries and full damage integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped damage reduction parity passed: {report_path}')


if __name__=='__main__':main()
