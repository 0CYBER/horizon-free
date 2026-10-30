#!/usr/bin/env python3
"""Compare registered damage sources through native server-player combat."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
import math
import zipfile
try:
    from . import lab, test_rust_parity as base, test_server_player_combat_parity as combat
except ImportError:
    import lab
    import test_rust_parity as base
    import test_server_player_combat_parity as combat


def corpus(catalog,damage):
    import copy
    rng=random.Random(0x524547434f4d4241)
    names=[d['id'] for d in damage['types']]
    for f in combat.corpus(catalog):
        for o in f['ops']:o.update(source_type=rng.choice(names),direct=rng.randrange(4))
        yield f
    template=next(combat.corpus(catalog));base=copy.deepcopy(template['ops'][0]);base.update(damage='41000000',blocked='00000000',fail=-1,removed=False,permanent=False,temporary=False,ability_invulnerable=False,client=False,fire_immune=False,fall_immune=False,rule_drowning=True,rule_fall=True,rule_fire=True,rule_freeze=True,loaded=True,changing=False,creative=False,sleeping=False,can_harm=True,weapon=True,owner=1,difficulties=[2,2,2],reset_cooldown=True)
    for name in names:
        for direct in range(4):
            for attacker in range(4):
                for absorption in ['00000000','40800000']:
                    f=copy.deepcopy(template);f.update(health='41a00000',absorption=absorption,exhaustion='00000000',cooldown=0,last_hurt='00000000',dead=False);f['ops']=[dict(base,source_type=name,direct=direct,attacker=attacker,damage=struct.pack('>f',v).hex()) for v in [1,3,8,20]];yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-registered-player-combat-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    damage_dir=build/'damage';damage_dir.mkdir(exist_ok=True)
    with zipfile.ZipFile(jars['server']) as archive:
        for file,prefix in [('types.json','data/minecraft/damage_type/'),('tags.json','data/minecraft/tags/damage_type/')]:
            resources={'minecraft:'+n[len(prefix):-5]:archive.read(n).decode('utf-8') for n in sorted(archive.namelist()) if n.startswith(prefix) and n.endswith('.json')};lab.write_json(damage_dir/file,resources)

    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaRegisteredPlayerCombatOracle.java'
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'
    health_helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerHealthOracle.java'
    damage_helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDamageSourceOracle.java'
    sources=[oracle,helper,health_helper,damage_helper,lab.ROOT/'tools/test_server_player_combat_parity.py',Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_player_hurt_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/registered_player_combat_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper,health_helper,damage_helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaRegisteredPlayerCombatOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'catalog.json').read_text());damage=json.loads((damage_dir/'catalog.json').read_text());fixtures=list(corpus(states,damage));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'catalog.json',build/'ids.json',build/'slots.json',build/'head.json',damage_dir/'catalog.json',damage_dir/'types.json',damage_dir/'tags.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'registered_player_combat_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} registered player combat output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'All 51 bundled registered damage types and 36 native-resolved tags drive the actual ServerPlayer damage chain, immunity, difficulty scaling, armor/resistance, exhaustion, health/absorption and hurt state. Direct/causing entity identity and creative-source abilities use real DamageSource methods. Equipment is unenchanted; virtual flags/rule values, blocking amount, durability, PvP permission, attribution/effect/knockback/death services and empty criterion listeners are controlled. No full entity lifecycle, enchantments, death messages or playable integration claim.'}
    lab.write_json(report_path,report);print(f'Scoped registered player combat parity passed: {report_path}')


if __name__=='__main__':main()
