#!/usr/bin/env python3
"""Compare registered entity immersion, support and current composition."""
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


try:
    from . import test_entity_fluid_parity as immersion
except ImportError:
    import test_entity_fluid_parity as immersion

def corpus(info):
    import copy
    seed=next(immersion.corpus())['ops'][0]
    defaults=info['defaults'];states=info['states'];air=defaults['minecraft:air'];water=defaults['minecraft:water'];lava=defaults['minecraft:lava'];stone=defaults['minecraft:stone']
    def bits(v):return struct.pack('>d',float(v)).hex()
    def cell(state):return dict(seed['default'],state=state)
    def op(**kw):
        v=copy.deepcopy(seed);v.update(mode='registered',default=cell(air),cells={},eye=bits(.9),block_tag=0);v.update(kw);return v
    def row(ops,player=False,order=(0,1,2)):return dict(ops=ops,player=player,order=order)
    for state in range(len(states)):
        for cells in [{'0 0 0':cell(state)},{'0 0 0':cell(water),'0 1 0':cell(state)}]:
            yield row([op(cells=cells),op(mode='apply',tag=0),op(mode='apply',tag=1),op(bounds=None),op(bounds=None)])
    special=[defaults['minecraft:'+n] for n in ['air','stone','water','lava','ice','frosted_ice','blue_ice','packed_ice','shulker_box','moving_piston','scaffolding','bamboo','oak_slab','powder_snow','pointed_dripstone']]
    liquids=[i for i,s in enumerate(states) if s['id'] in ['minecraft:water','minecraft:lava']]
    rng=random.Random(0x454e54524547464c)
    for _ in range(4000):
        ops=[]
        for i in range(rng.randrange(1,5)):
            cells={f'{x} {y} {z}':cell(rng.choice(special+liquids)) for x in range(-1,3) for y in range(-1,3) for z in range(-1,3)}
            ops.append(op(bounds=[bits(v) for v in [0,0,0,2,2,2]],cells=cells,base_y=bits(rng.choice([-.1,0,.1,.4,1])),eye=bits(rng.choice([0,.1,.4,8/9,1,1.5,2])),block_tag=rng.randrange(3),tags=[rng.randrange(32) for _ in range(4)],ignore=bool(rng.randrange(2)),fail=rng.choice([-1,-1,*range(50)]),mutate_at=rng.choice([-1,-1,*range(35)])))
            ops.extend([op(mode='apply',tag=0),op(mode='apply',tag=1)])
        yield row(ops,bool(rng.randrange(2)),rng.sample(range(4),rng.randrange(5)))
    for root in liquids:
        for fail in range(-1,35):
            for sturdy in [air,stone,defaults['minecraft:shulker_box']]:
                yield row([op(cells={'0 0 0':cell(root),'0 1 0':cell(sturdy),'1 -1 0':cell(root)},fail=fail),op(mode='apply',tag=0),op(mode='apply',tag=1)])


def main():
    lock,jars,libraries=base.verify_artifacts();build=lab.LOCAL/'rust-registered-entity-fluid-parity';build.mkdir(exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    root=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity';oracles=[root/(n+'.java') for n in ['VanillaRegisteredEntityFluidOracle','VanillaEntityFluidOracle','VanillaRegistryFluidOracle','VanillaHeightmapOracle','VanillaBlockSupportOracle','VanillaCollisionOracle','VanillaBlockStateCodecOracle']]
    sources=[*oracles,Path(__file__),lab.ROOT/'tools/test_entity_fluid_parity.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_worldgen_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/examples/probe/entity_fluid_fixture.rs']
    for crate in ['horizon-core','horizon-world']:sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested=base.source_hashes(sources);tags=worldgen.tag_resources(jars['server'])
    if tags!=worldgen.tag_resources(jars['client']):raise AssertionError('Native tags differ between artifacts')
    lab.write_json(build/'tags.json',tags);classes=build/'classes';classes.mkdir(exist_ok=True);dependencies=os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,mode):run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.VanillaRegisteredEntityFluidOracle',mode,build],side+'-'+mode)
    java('server','export');info=json.loads((build/'facts.json').read_text());count=0
    if len(info['states'])!=35723:raise AssertionError('Incomplete pinned registry')
    with (build/'inputs.txt').open('w') as output:
        for f in corpus(info):output.write(json.dumps(f,separators=(',',':'))+'\n');count+=1
    fixture_paths=[build/n for n in ['inputs.txt','facts.json','fluid-info.json','block-registry.nbt','cached-support.nbt','tags.json']];fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust');metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,'entity_fluid_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side);matched=base.compare(build/(side+'.txt'),build/'rust.txt');references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),output_sha256=lab.digest(build/(side+'.txt'),'sha256'),matched_output_lines=matched));print(f'{side}: {matched:,} registered entity fluid output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report=dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),fixtures=count,registry_states=len(info['states']),sources_sha256=tested,fixtures_sha256=fixtures,rust_output_sha256=lab.digest(build/'rust.txt','sha256'),references=references,scope='Actual EntityFluidInteraction update and current application with registered contained fluids, native height/camera/current calculations and cached/dynamic block support. All registered states in immersed cells and above-eye support cells; mixed water/lava grids, ordered/reloaded tags, retained trackers, current velocities, callback mutations and failures. Chunk presence/section wet flags and cell storage are controlled; block entities are absent. Live chunk/entity lifecycle, fluid propagation and complete playable gameplay remain pending.')
    lab.write_json(report_path,report);print(f'Scoped registered entity fluid parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
