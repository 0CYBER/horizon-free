#!/usr/bin/env python3
"""Compare composed server-player damage, cooldown, health and hunger with vanilla."""
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
    from . import lab, test_rust_parity as base, test_player_hurt_parity as hurt
except ImportError:
    import lab
    import test_rust_parity as base
    import test_player_hurt_parity as hurt


def storage_corpus(catalog):
    import copy
    def bits(v):return struct.pack('>f',v).hex()
    def double(v):return struct.pack('>d',v).hex()
    rng=random.Random(0x434f4d424154)
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,5,10,20,1e30,float('inf'),float('nan')]
    def extend(f,randomize=False):
        f=copy.deepcopy(f);f.update(health=bits(20),absorption=bits(4),exhaustion=bits(0),dead=False,attributes=[dict(base=double(v),amount=double(0),operation=0) for v in [15,8,20,20]])
        if randomize:
            for k in ['health','absorption','exhaustion']:f[k]=bits(rng.choice(values))
            f['dead']=rng.randrange(8)==0
            for a in f['attributes']:a.update(base=double(rng.choice(values)),amount=double(rng.choice(values)),operation=rng.randrange(3))
        for o in f['ops']:
            o.update(weapon=False,resistance=False,amplifier=0,break_armor=False,armor_after=double(0),food_exhaustion=bits(.1),pack=False,pack_callback=False,client=False,bypass_armor=False,bypass_effects=False,bypass_resistance=False,bypass_enchantments=False)
            if randomize:
                for k in ['weapon','resistance','break_armor','pack','pack_callback','client','bypass_armor','bypass_effects','bypass_resistance','bypass_enchantments']:o[k]=bool(rng.randrange(2))
                o.update(amplifier=rng.choice([-2147483648,-2,-1,0,1,4,5,2147483647]),armor_after=double(rng.choice(values)),food_exhaustion=bits(rng.choice(values)),fail=rng.choice([-1,-1,-1,*range(65)]))
        return f
    for i,f in enumerate(hurt.corpus()):yield extend(f,i%2==0)
    template=extend(next(hurt.corpus()))
    clean=copy.deepcopy(template['ops'][0]);clean.update(damage=bits(8),blocked=bits(0),ability_invulnerable=False,scales=False,attacker=0,owner=0,can_harm=True,difficulties=[2,2,2],fail=-1,reset_cooldown=True)
    for health in values:
        for absorption in values:
            f=copy.deepcopy(template);f.update(health=bits(health),absorption=bits(absorption),cooldown=0,last_hurt=bits(0));f['ops']=[dict(clean,damage=bits(v),pack=i%2==0) for i,v in enumerate([1,2,4,8,16,32,64])];yield f
    for fail in range(-1,66):
        for absorption in [0,4,40]:
            f=copy.deepcopy(template);f.update(absorption=bits(absorption),cooldown=0,last_hurt=bits(0));f['ops']=[dict(clean,fail=fail,pack_callback=True,break_armor=True,resistance=True,amplifier=0),dict(clean,damage=bits(25),pack=True)];yield f


def corpus(catalog):
    rng=random.Random(0x494d4d554e45434f)
    for i,f in enumerate(storage_corpus(catalog)):
        for o in f['ops']:
            o.update(removed=False,permanent=o['invulnerable'],temporary=False,creative=False,fire_immune=False,fall_immune=False,fall=False,drowning=False,rule_drowning=True,rule_fall=True,rule_fire=True,rule_freeze=True,changing=False,pearl=False,loaded=True)
            if i%3==0:
                for k in ['removed','temporary','creative','fire_immune','fall_immune','fall','drowning','changing','pearl']:o[k]=rng.randrange(8)==0
                for k in ['rule_drowning','rule_fall','rule_fire','rule_freeze','loaded']:o[k]=rng.randrange(4)!=0
        yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-server-player-combat-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaServerPlayerCombatOracle.java'
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'
    health_helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerHealthOracle.java'
    sources=[oracle,helper,health_helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_player_hurt_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/server_player_combat_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper,health_helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaServerPlayerCombatOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'catalog.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'catalog.json',build/'ids.json',build/'slots.json',build/'head.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'server_player_combat_fixture',build],'rust')
    run([binary,'server_player_combat_fixture',build,'--live'],'rust-live')
    base.compare(build/'rust.txt',build/'rust-live.txt');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} server player combat output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'live_rust_output_sha256':lab.digest(build/'rust-live.txt','sha256'),'references':references,'scope':'Both EntityAttributes and LiveAttributes storage bindings produce the same native traces. Actual ServerPlayer outer hurt through Player/LivingEntity gates and actual Player inner damage, native armor/resistance reduction, registered modified attributes, synchronized health/absorption and FoodData exhaustion. Repeated hits, cooldown/state writes, death rejection, callbacks and numeric edges. Actual inherited invulnerability predicates include virtual flags, empty-equipment enchantment immunity, damage gamerules and client-loading/dimension guards; blocking amount, durability, effects, PvP permission, attribution, knockback, death services and criterion listeners remain controlled; enchanted items, registered sources and complete entity/game lifecycle remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped server player combat parity passed: {report_path}')


if __name__=='__main__':main()
