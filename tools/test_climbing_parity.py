#!/usr/bin/env python3
"""Compare registered climbing and trapdoor ladder behavior."""
import itertools
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(info):
    defaults=info['defaults'];air=defaults['minecraft:air'];ladder=defaults['minecraft:ladder'];stone=defaults['minecraft:stone']
    def op(state,**kw):
        f=dict(state=state,below=ladder,position=[17,-64,-33],spectator=False,gliding=False,fail=-1,mutate_at=-1);f.update(kw);return f
    def row(state,ct=0,gt=0,**kw):return dict(climb_tags=ct,glide_tags=gt,ops=[op(state,**kw),op(air)])
    for tags in range(3):
        for state in range(info['states']):
            for gliding in [False,True]:yield row(state,ct=tags,gt=tags,gliding=gliding)
    for state in info['trapdoor_states']:
        for below in [air,stone,*info['ladder_states']]:yield row(state,ct=1,gt=1,below=below)
    special=[defaults['minecraft:'+n] for n in ['air','stone','ladder','scaffolding','vine','oak_trapdoor','iron_trapdoor','powder_snow']]
    for ct,gt,state,spectator,gliding in itertools.product(range(3),range(3),special,[False,True],[False,True]):yield row(state,ct=ct,gt=gt,spectator=spectator,gliding=gliding)
    for state in special+[s for s in info['trapdoor_states'][:64]]:
        for fail in range(-1,6):
            for mutate_at in range(-1,6):yield row(state,fail=fail,mutate_at=mutate_at,position=[2147483647,-2147483648,-2147483648])
    rng=random.Random(0x434c494d42)
    for _ in range(4000):
        yield dict(climb_tags=rng.randrange(3),glide_tags=rng.randrange(3),ops=[op(rng.randrange(info['states']),below=rng.choice(info['ladder_states']+[air,stone]),spectator=bool(rng.randrange(2)),gliding=bool(rng.randrange(2)),fail=rng.randrange(-1,6),mutate_at=rng.randrange(-1,6),position=rng.choice([[0,0,0],[17,-64,-33],[2147483647,-2147483648,-2147483648]])) for _ in range(4)])


def main():
    lock,jars,libraries=base.verify_artifacts();build=lab.LOCAL/'rust-climbing-parity';build.mkdir(exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    root=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity';oracles=[root/(n+'.java') for n in ['VanillaClimbingOracle','VanillaHeightmapOracle']]
    sources=[*oracles,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_worldgen_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/examples/probe/climbing_fixture.rs']
    for crate in ['horizon-core','horizon-world']:sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested=base.source_hashes(sources);tags=worldgen.tag_resources(jars['server'])
    if tags!=worldgen.tag_resources(jars['client']):raise AssertionError('Native tags differ between artifacts')
    lab.write_json(build/'tags.json',tags);classes=build/'classes';classes.mkdir(exist_ok=True);dependencies=os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,mode):run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.VanillaClimbingOracle',mode,build],side+'-'+mode)
    java('server','export');info=json.loads((build/'info.json').read_text());count=0
    if info['states']!=35723:raise AssertionError('Incomplete pinned registry')
    with (build/'inputs.txt').open('w') as output:
        for f in corpus(info):output.write(json.dumps(f,separators=(',',':'))+'\n');count+=1
    fixture_paths=[build/n for n in ['inputs.txt','info.json','block-registry.nbt','tags.json']];fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust');metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,'climbing_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side);matched=base.compare(build/(side+'.txt'),build/'rust.txt');references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),output_sha256=lab.digest(build/(side+'.txt'),'sha256'),matched_output_lines=matched));print(f'{side}: {matched:,} climbing output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report=dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),fixtures=count,registry_states=info['states'],sources_sha256=tested,fixtures_sha256=fixtures,rust_output_sha256=lab.digest(build/'rust.txt','sha256'),references=references,scope='Actual registered LivingEntity.onClimbable and trapdoorUsableAsLadder with native CLIMBABLE/CAN_GLIDE_THROUGH tags. Every registered state, natural/empty/universal tag snapshots, all trapdoor states against every ladder facing, gliding and spectator guards, captured positions, retained climb history, integer-coordinate wrapping, mutations and failures. Rust binding validates members/properties/state IDs. Entity/world queries are controlled; live movement binding, equipment-based glide eligibility and complete gameplay remain pending.')
    lab.write_json(report_path,report);print(f'Scoped climbing parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
