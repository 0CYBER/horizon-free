#!/usr/bin/env python3
"""Compare registered fall-reset world rays with native blocks, fluids and tags."""
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


def fluid_tags(jar):
    with zipfile.ZipFile(jar) as archive:
        prefix='data/minecraft/tags/fluid/'
        return {'minecraft:'+n[len(prefix):-5]:archive.read(n).decode('utf-8') for n in sorted(archive.namelist()) if n.startswith(prefix) and n.endswith('.json')}


def corpus(info):
    def bits(v):return struct.pack('>d',float(v)).hex()
    defaults=info['defaults'];water=defaults['minecraft:water'];lava=defaults['minecraft:lava']
    def op(above=0,p=(0,0,0),ray=0,fail=-1):
        a,b=[([.5,.5,.5],[2,.5,.5]),([-.2,.5,.5],[1.2,.5,.5]),([.5,1.5,.5],[.5,-1,.5]),([-.5,-.25,.5],[1.5,1.75,.5]),([.5,.5,.5],[.5,.5,.5])][ray]
        return dict(above=above,fail=fail,**{'from':[bits(v+p[i]) for i,v in enumerate(a)],'to':[bits(v+p[i]) for i,v in enumerate(b)]})
    def row(state,**kw):
        f=dict(state=state,position=[0,0,0],block_tags=0,fluid_tags=0,server=True,entity=True,player=True,delay=0,ops=[op(),op(water),op()]);f.update(kw);return f
    for tags in range(3):
        for state in range(info['states']):yield row(state,block_tags=tags)
    special=[defaults['minecraft:'+n] for n in ['air','stone','water','lava','cobweb','sweet_berry_bush','slime_block','honey_block','hay_block','powder_snow','end_gateway','end_portal','nether_portal','cauldron','water_cauldron','composter','hopper','scaffolding']]
    for state,bt,ft,entity,player,server,delay in itertools.product(special,range(3),range(3),[False,True],[False,True],[False,True],[-1,0,1,80]):
        yield row(state,block_tags=bt,fluid_tags=ft,entity=entity,player=player,server=server,delay=delay,ops=[op(0,ray=1),op(water,ray=2),op(lava,ray=3)])
    for state,ft,fail in itertools.product(special,range(3),range(-1,35)):
        yield row(state,fluid_tags=ft,ops=[op(0,ray=1,fail=fail),op(water,ray=2,fail=fail),op(0)])
    rng=random.Random(0x46414c4c524159)
    for _ in range(6000):
        p=rng.choice([(0,0,0),(17,-64,-33),(30000000,0,-30000000)])
        yield row(rng.randrange(info['states']),position=p,block_tags=rng.randrange(3),fluid_tags=rng.randrange(3),entity=bool(rng.randrange(2)),player=bool(rng.randrange(2)),server=bool(rng.randrange(2)),delay=rng.choice([-1,0,1,80]),ops=[op(rng.choice([0,water,lava,rng.randrange(info['states'])]),p,ray=rng.randrange(5),fail=rng.randrange(-1,20)) for _ in range(4)])


def main():
    lock,jars,libraries=base.verify_artifacts();build=lab.LOCAL/'rust-registry-fall-ray-parity';build.mkdir(exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    root=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity';oracles=[root/(n+'.java') for n in ['VanillaRegistryFallRayOracle','VanillaInteractionShapeOracle','VanillaCollisionOracle','VanillaHeightmapOracle']]
    sources=[*oracles,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_worldgen_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/examples/probe/registry_fall_ray_fixture.rs']
    for crate in ['horizon-core','horizon-world']:sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested=base.source_hashes(sources);tags=worldgen.tag_resources(jars['server']);fluids=fluid_tags(jars['server'])
    if tags!=worldgen.tag_resources(jars['client']) or fluids!=fluid_tags(jars['client']):raise AssertionError('Native tags differ between artifacts')
    lab.write_json(build/'tags.json',tags);lab.write_json(build/'fluid-tags.json',fluids);classes=build/'classes';classes.mkdir(exist_ok=True);dependencies=os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,mode):run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.VanillaRegistryFallRayOracle',mode,build],side+'-'+mode)
    java('server','export');info=json.loads((build/'info.json').read_text());count=0
    if info['states']!=35723:raise AssertionError('Incomplete pinned registry')
    with (build/'inputs.txt').open('w') as output:
        for f in corpus(info):output.write(json.dumps(f,separators=(',',':'))+'\n');count+=1
    fixture_paths=[build/n for n in ['inputs.txt','info.json','block-registry.nbt','interaction-shapes.nbt','tags.json','fluid-tags.json','bound-tags.json']];fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust');metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,'registry_fall_ray_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side);matched=base.compare(build/(side+'.txt'),build/'rust.txt');references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),output_sha256=lab.digest(build/(side+'.txt'),'sha256'),matched_output_lines=matched));print(f'{side}: {matched:,} registered fall-ray output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report=dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),fixtures=count,registry_states=info['states'],sources_sha256=tested,fixtures_sha256=fixtures,rust_output_sha256=lab.digest(build/'rust.txt','sha256'),references=references,scope='Actual BlockGetter.clip with registered block/interaction/fluid shapes, FALLDAMAGE_RESETTING/WATER context and native tag reloads. Every block state runs natural, empty and universal reset tags; additional fluid tag modes, entity/player/server distinctions, portal delays, multiple ray positions, above-fluid changes, cache reuse and failures compare complete traces/results. Binding rejects unknown tag members and invalid block IDs. World block/fluid reads use controlled snapshots; live resident-world binding, actual player storage, movement integration and complete gameplay remain pending.')
    lab.write_json(report_path,report);print(f'Scoped registered fall ray parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
