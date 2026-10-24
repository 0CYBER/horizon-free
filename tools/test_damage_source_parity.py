#!/usr/bin/env python3
"""Compare registered damage-source identity and gameplay queries with vanilla."""
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
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(catalog):
    def bits(v):return struct.pack('>d',v).hex()
    names=[d['id'] for d in catalog['types']]
    def state(**kw):
        s=dict(weapons=[False,True,False,True,False],creative=[False,False,False,True,False],positions=[[bits(i*.25),bits(-i),bits(i+.75)] for i in range(5)],virtual_entity=4);s.update(kw);return s
    def row(name,direct=0,causing=0,constructor=0,**kw):
        f=dict(type=name,direct=direct,causing=causing,constructor=constructor,position=[bits(-0.),bits(10),bits(-20)],initial=state(),after=state(),virtual=False,fail=-1,mutate_at=-1,check_type=name);f.update(kw);return f
    for name in names:
        for direct in range(5):
            for causing in range(5):
                for constructor in range(4):yield row(name,direct,causing,constructor)
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-300,.1,1,5,10,20,1e30,float('inf'),float('nan')]
    rng=random.Random(0x44414d4147455352)
    def random_state():return state(weapons=[bool(rng.randrange(2)) for _ in range(5)],creative=[bool(rng.randrange(2)) for _ in range(5)],positions=[[bits(rng.choice(values)) for _ in range(3)] for _ in range(5)],virtual_entity=rng.randrange(5))
    for _ in range(10000):yield row(rng.choice(names),rng.randrange(5),rng.randrange(5),rng.randrange(4),initial=random_state(),after=random_state(),virtual=bool(rng.randrange(2)),fail=rng.randrange(-1,5),mutate_at=rng.randrange(-1,5),position=[bits(rng.choice(values)) for _ in range(3)],check_type=rng.choice(names+['minecraft:unknown']))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-damage-source-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    with zipfile.ZipFile(jars['server']) as archive:
        for file,prefix in [('types.json','data/minecraft/damage_type/'),('tags.json','data/minecraft/tags/damage_type/')]:
            resources={'minecraft:'+n[len(prefix):-5]:archive.read(n).decode('utf-8') for n in sorted(archive.namelist()) if n.startswith(prefix) and n.endswith('.json')};lab.write_json(build/file,resources)

    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDamageSourceOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/damage_source_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaDamageSourceOracle',mode,path],side+'-'+mode)
    java('server','export',build);catalog=json.loads((build/'catalog.json').read_text());fixtures=list(corpus(catalog));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'catalog.json',build/'types.json',build/'tags.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'damage_source_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} damage source output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Native DamageType codec export and TagLoader resolution for all bundled vanilla damage definitions/tags; ordinary DamageSource constructor identity, difficulty scaling, exhaustion, type/tag membership, weapon/direct-position access and virtual creative-source dispatch. Entity position/weapon/ability queries are controlled, including mutation and failures. Catalog indices are local, not network IDs; live registry reload, damage definition codecs in Rust, death-message generation and combat registry binding remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped damage source parity passed: {report_path}')


if __name__=='__main__':main()
