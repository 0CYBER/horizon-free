#!/usr/bin/env python3
"""Compare registered fluid contents, height and current support composition."""
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
    defaults=info['defaults'];states=info['states'];air=defaults['minecraft:air'];water=defaults['minecraft:water'];lava=defaults['minecraft:lava']
    flowing=[i for i,s in enumerate(states) if s['id'] in ['minecraft:water','minecraft:lava']]
    falling_water=next(i for i in flowing if states[i]['id']=='minecraft:water' and states[i]['properties']['level']=='8')
    falling_lava=next(i for i in flowing if states[i]['id']=='minecraft:lava' and states[i]['properties']['level']=='8')
    def row(root,**kw):
        f=dict(mode='flow',root=root,default=air,cells={},position=[0,0,0],fail=-1,direction=2,tag=0);f.update(kw);return f
    for i in range(len(states)):
        for mode in ['flow','height','camera']:yield row(i,mode=mode,default=water)
        for direction in range(6):yield row(water,mode='face',direction=direction,default=i)
        yield row(falling_water,cells={'0 0 -1':i,'1 0 0':water,'-1 -1 0':lava})
        yield row(water,mode='camera',default=i)
    for tag in range(3):
        for i in defaults.values():
            for root in [falling_water,falling_lava]:yield row(root,tag=tag,cells={'0 0 -1':i,'0 -1 -1':root})
    special=[defaults['minecraft:'+n] for n in ['air','stone','ice','frosted_ice','packed_ice','blue_ice','water','lava','shulker_box','moving_piston','scaffolding','oak_slab','powder_snow','bamboo','pointed_dripstone']]
    for i in special:
        for root in [falling_water,falling_lava]:
            for mode in ['flow','height','camera','face']:
                for fail in range(-1,30):yield row(root,mode=mode,default=i,fail=fail)
    rng=random.Random(0x524547464c554944)
    for _ in range(5000):
        p=rng.choice([(0,0,0),(17,-64,-33),(2147483647,-2147483648,-2147483648)])
        def wrap(v):return (v+2**31)%2**32-2**31
        cells={' '.join(str(wrap(a+b)) for a,b in zip(p,(dx,y,dz))):rng.choice(special+flowing) for dx,dz in [(0,-1),(1,0),(0,1),(-1,0)] for y in [-1,0,1]}
        yield row(rng.choice(flowing),cells=cells,default=rng.choice(special),position=p,fail=rng.choice([-1,-1,*range(35)]),tag=rng.randrange(3))


def main():
    lock,jars,libraries=base.verify_artifacts();build=lab.LOCAL/'rust-registry_fluid-parity';build.mkdir(exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    root=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity';oracles=[root/(n+'.java') for n in ['VanillaRegistryFluidOracle','VanillaHeightmapOracle','VanillaBlockSupportOracle','VanillaCollisionOracle','VanillaBlockStateCodecOracle']]
    sources=[*oracles,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_worldgen_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/examples/probe/registry_fluid_fixture.rs']
    for crate in ['horizon-core','horizon-world']:sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested=base.source_hashes(sources);tags=worldgen.tag_resources(jars['server'])
    if tags!=worldgen.tag_resources(jars['client']):raise AssertionError('Native tags differ between artifacts')
    lab.write_json(build/'tags.json',tags);classes=build/'classes';classes.mkdir(exist_ok=True);dependencies=os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,mode):run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.VanillaRegistryFluidOracle',mode,build],side+'-'+mode)
    java('server','export');info=json.loads((build/'facts.json').read_text());count=0
    if len(info['states'])!=35723:raise AssertionError('Incomplete pinned registry')
    with (build/'inputs.txt').open('w') as output:
        for f in corpus(info):output.write(json.dumps(f,separators=(',',':'))+'\n');count+=1
    fixture_paths=[build/n for n in ['inputs.txt','facts.json','fluid-info.json','block-registry.nbt','cached-support.nbt','tags.json']];fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust');metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,'registry_fluid_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side);matched=base.compare(build/(side+'.txt'),build/'rust.txt');references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),output_sha256=lab.digest(build/(side+'.txt'),'sha256'),matched_output_lines=matched));print(f'{side}: {matched:,} registry_fluid output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report=dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),fixtures=count,registry_states=len(info['states']),sources_sha256=tested,fixtures_sha256=fixtures,rust_output_sha256=lab.digest(build/'rust.txt','sha256'),references=references,scope='Actual registered fluid height/camera-height/current vectors and solid-face selection using all native block states, BLOCKS_FLUID_FLOW tags and IceBlock class identities. Rust composes registered contained fluids with cached/dynamic block support, preserving world query order, directional walls, waterlogged contents, coordinate wrapping and failures. Block storage is controlled and block entities are absent; live entity/world binding, active block-entity lifecycle, fluid spreading and complete gameplay remain pending.')
    lab.write_json(report_path,report);print(f'Scoped registry_fluid parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
