#!/usr/bin/env python3
"""Compare native armor, toughness and magic protection arithmetic with vanilla."""
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
    def row(damage,armor,toughness,**kw):
        f=dict(mode='armor',damage=bits(damage),armor=bits(armor),toughness=bits(toughness),weapon=False,server=False,after_weapon=True,after_server=True,empty=True,fail=-1,mutate_at=-1);f.update(kw);return f
    values=[-float('inf'),-1e30,-25,-20,-8,-1,-0.,0.,1e-40,.1,1,2,4,8,20,25,40,1e30,float('inf'),float('nan')]
    for damage in values:
        for armor in values:
            yield row(damage,armor,0,mode='magic')
            for toughness in values:
                for weapon,server,empty in [(False,False,True),(True,False,True),(True,True,True),(True,True,False)]:yield row(damage,armor,toughness,weapon=weapon,server=server,empty=empty)
    for weapon in [False,True]:
        for server in [False,True]:
            for after_weapon in [False,True]:
                for after_server in [False,True]:
                    for fail in range(-1,3):
                        for mutation in range(-1,3):yield row(17,20,8,weapon=weapon,server=server,after_weapon=after_weapon,after_server=after_server,fail=fail,mutate_at=mutation)
    rng=random.Random(0x41524d4f52)
    for _ in range(8000):
        f=row(0,0,0,weapon=bool(rng.randrange(2)),server=bool(rng.randrange(2)),empty=bool(rng.randrange(2)),mode=rng.choice(['armor','magic']))
        for key in ['damage','armor','toughness']:f[key]=f'{rng.getrandbits(32):08x}'
        yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-combat-rules-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCombatRulesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/combat_rules_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaCombatRulesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'combat_rules_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} combat rules output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual CombatRules armor/toughness and magic protection arithmetic; null/empty/unenchanted weapons, nonserver/server branches, native no-enchantment helper with explicitly empty stack components, query order/mutations/failures, nonfinite/signed-zero and random float bit patterns. The armor-effectiveness enchantment hook is exposed but enchanted components, damage tags/equipment and the full damage pipeline remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped combat rules parity passed: {report_path}')


if __name__=='__main__':main()
