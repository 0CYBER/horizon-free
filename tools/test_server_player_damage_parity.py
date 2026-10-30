#!/usr/bin/env python3
"""Compare registered server player damage and synchronized absorption with vanilla."""
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


def corpus(catalog):
    def bits(v):return struct.pack('>f',v).hex()
    def double(v):return struct.pack('>d',v).hex()
    def op(damage=8,**kw):
        o=dict(damage=bits(damage),fail=-1,invulnerable=False,ability_invulnerable=False,client=False,weapon=False,resistance=False,amplifier=0,break_armor=False,armor_after=double(0),food_exhaustion=bits(.1),pack=False,pack_callback=False,**{'bypass_'+k:False for k in ['armor','effects','resistance','enchantments']});o.update(kw);return o
    def row(ops,health=20,absorption=4,armor=15,toughness=8,maximum=20,max_absorption=20,exhaustion=0,amounts=None,operations=None):
        return dict(health=bits(health),absorption=bits(absorption),exhaustion=bits(exhaustion),attributes=[dict(base=double(v),amount=double((amounts or [0]*4)[i]),operation=(operations or [0]*4)[i]) for i,v in enumerate([armor,toughness,maximum,max_absorption])],ops=ops)
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,5,10,20,1e30,float('inf'),float('nan')]
    for damage in values:
        for absorption in values:
            for armor in [-1,0,1,15,30,100,float('nan')]:
                yield row([op(damage),op(damage,weapon=True,pack=True),op(damage,resistance=True)],absorption=absorption,armor=armor)
    for mask in range(16):
        for amp in [-2147483648,-2,-1,0,1,4,5,2147483647]:
            for fail in range(-1,28):
                yield row([op(19,amplifier=amp,resistance=True,fail=fail,break_armor=True,pack_callback=True,**{'bypass_'+k:bool(mask&(1<<i)) for i,k in enumerate(['armor','effects','resistance','enchantments'])}),op(7,pack=True)])
    rng=random.Random(0x44414d414745)
    for _ in range(2000):
        ops=[]
        for __ in range(rng.randrange(1,15)):
            o=op(rng.choice(values),amplifier=rng.choice([-2147483648,-2,-1,0,1,3,4,5,2147483647]),armor_after=double(rng.choice(values)),food_exhaustion=bits(rng.choice(values)),fail=rng.choice([-1,-1,-1,*range(28)]))
            for k in ['invulnerable','ability_invulnerable','client','weapon','resistance','break_armor','pack','pack_callback','bypass_armor','bypass_effects','bypass_resistance','bypass_enchantments']:o[k]=bool(rng.randrange(2))
            ops.append(o)
        yield row(ops,**{k:rng.choice(values) for k in ['health','absorption','armor','toughness','maximum','max_absorption','exhaustion']},amounts=[rng.choice(values) for _ in range(4)],operations=[rng.randrange(3) for _ in range(4)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-server-player-damage-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaServerPlayerDamageOracle.java'
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'
    health_helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlayerHealthOracle.java'
    sources=[oracle,helper,health_helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/server_player_damage_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper,health_helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaServerPlayerDamageOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'catalog.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'catalog.json',build/'ids.json',build/'slots.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'server_player_damage_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} server player damage output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Native ServerPlayer inner damage with actual armor/magic reduction, registered modified attributes, Player health/absorption synchronized storage and FoodData exhaustion. Repeated hits, armor attribute mutation, empty equipment, resistance, bypass tags, callback failures and packet drains. Equipment durability, effects, source tags, combat/stat/event services are controlled; outer hurt gates, enchanted equipment and full player lifecycle remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped server player damage parity passed: {report_path}')


if __name__=='__main__':main()
