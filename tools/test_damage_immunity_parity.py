#!/usr/bin/env python3
"""Compare native entity/living/player damage immunity with vanilla."""
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
    names=['removed','permanent','temporary','bypass','creative','fire','fire_immune','fall','fall_immune','drowning','freezing','rule_drowning','rule_fall','rule_fire','rule_freeze','changing','pearl','loaded']
    modes=['entity','base','living','player','server']
    def state(**kw):
        s={n:False for n in names};s.update(rule_drowning=True,rule_fall=True,rule_fire=True,rule_freeze=True,loaded=True);s.update(kw);return s
    def row(s,mode='server',**kw):
        f=dict(initial=s,after=s,mode=mode,fail=-1,mutate_at=-1);f.update(kw);return f
    for mask in range(512):
        s=state(**{n:bool(mask&(1<<i)) for i,n in enumerate(names[:9])})
        for mode in modes:yield row(s,mode)
    for tags in range(16):
        for rules in range(16):
            for server in range(8):
                yield row(state(**{n:bool(tags&(1<<i)) for i,n in enumerate(['drowning','fall','fire','freezing'])},**{'rule_'+n:bool(rules&(1<<i)) for i,n in enumerate(['drowning','fall','fire','freeze'])},**{n:bool(server&(1<<i)) for i,n in enumerate(['changing','pearl','loaded'])}))
    rng=random.Random(0x494d4d554e495459)
    for _ in range(10000):
        yield row(state(**{n:rng.randrange(4)==0 for n in names}),rng.choice(modes),after=state(**{n:bool(rng.randrange(2)) for n in names}),fail=rng.choice([-1,-1,-1,*range(24)]),mutate_at=rng.choice([-1,*range(24)]))
    for fail in range(-1,25):
        for mutate in range(-1,25):
            for mode in modes:
                yield row(state(drowning=True,changing=True,pearl=True),mode,after=state(fall=True,rule_fall=False,permanent=True),fail=fail,mutate_at=mutate)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-damage-immunity-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDamageImmunityOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/damage_immunity_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaDamageImmunityOracle',mode,path],side+'-'+mode)
    java('server','export',build);fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'slots.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'damage_immunity_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} damage immunity output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity invulnerability/base immunity, LivingEntity enchantment dispatch, Player gamerule immunity and ServerPlayer dimension/client-loading predicates. Controlled virtual flags, source tags and rules; real empty-equipment enchantment iteration. Compares short-circuit ordering, mixed tags, virtual state changes and callback failures. Registered source/rule bindings, nonempty enchantment immunity and full damage-pipeline integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped damage immunity parity passed: {report_path}')


if __name__=='__main__':main()
