#!/usr/bin/env python3
"""Compare world collision collection and entity step movement with pinned vanilla."""
import collections
import json
import os
from pathlib import Path
import platform
import subprocess
import struct
import random
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(facts):
    def bits(v):return struct.pack('>d',float(v)).hex()
    def row(state,delta=(1,-0.08,0),step=0.6,grounded=True,context=2,offset=0,fail=-1,extra=(),entities=(),border=None,box=None):
        blocks=[[x+offset,-1,z,facts['defaults']['minecraft:stone']] for x in range(-2,4) for z in range(-2,3)]
        blocks.append([offset,0,0,state]);blocks.extend(extra)
        return dict(chunks=[[x,z] for x in range((offset>>4)-1,(offset>>4)+2) for z in range(-1,2)],
                    blocks=blocks,context=context,y=bits(0),entities=[0,0],fail=fail,limit=100000,suffocating=False,
                    box=list(map(bits,box if box is not None else [-0.6+offset,0,0.2,-0.2+offset,1.8,0.8])),
                    movement=dict(delta=list(map(bits,delta)),step=bits(step),grounded=grounded,
                                  entities=[list(map(bits,b)) for b in entities],border=None if border is None else list(map(bits,border))))
    states=facts['states']
    for state in range(len(states)):
        yield row(state,context=state%4)
    defaults=facts['defaults']
    falling_surfaces=[('minecraft:stone_slab',0.5),('minecraft:oak_stairs',1.0),('minecraft:snow',0.125),
                      ('minecraft:oak_fence',1.5),('minecraft:iron_bars',1.0),('minecraft:soul_sand',0.875),
                      ('minecraft:farmland',0.9375)]
    for block_id,top in falling_surfaces:
        state=defaults[block_id]
        size=struct.unpack('>f',struct.pack('>f',0.98))[0]
        half=size/2
        bottom=top+0.1
        box=[0.5-half,bottom,0.5-half,0.5+half,bottom+size,0.5+half]
        yield row(state,delta=(0,-0.25,0),step=0,grounded=False,context=2,box=box)
    obstacles=[i for i,s in enumerate(states) if s['id'] in ['minecraft:oak_stairs','minecraft:stone_slab',
               'minecraft:snow','minecraft:scaffolding','minecraft:powder_snow','minecraft:water',
               'minecraft:shulker_box','minecraft:moving_piston','minecraft:piston','minecraft:soul_sand',
               'minecraft:mud','minecraft:farmland','minecraft:bamboo','minecraft:pointed_dripstone']]
    for state in obstacles:
        for step in [0,0.5,0.6,1.0,1.5]:
            for dy in [0,-0.08,-0.5,0.25]:
                yield row(state,delta=(1,dy,0),step=step)
    for delta in [(0,0,0),(-0.0,-0.0,-0.0),(1e-200,0,0),(1e-8,-1e-8,0),(1,0,0),(0,0,1),(1,-0.08,1)]:
        for state in [defaults['minecraft:stone_slab'],defaults['minecraft:oak_stairs']]:
            for fail in [-1,0,1,2,3,8,50,100,150,250]:yield row(state,delta=delta,fail=fail)
    rng=random.Random(0x4d4f56454d454e54)
    for index in range(5000):
        offset=rng.choice([-16,0,15]);state=rng.choice(obstacles)
        delta=[rng.choice([-1.25,-0.5,0,0.5,1.0,1.8]),rng.choice([-0.75,-0.08,0,0.25]),rng.choice([-1.0,-0.2,0,0.2,1.0])]
        entities=[]
        for _ in range(rng.randrange(4)):
            x=offset+rng.uniform(-1,2);z=rng.uniform(-1,2);y=rng.choice([-1,0,0.5,1.0,2.0]);entities.append([x,y,z,x+0.6,y+rng.choice([0.5,1.0,1.8]),z+0.6])
        border=[offset+0.8,-2,-2,offset+1.2,4,3] if index%4==0 else None
        extra=[[offset,2,0,defaults['minecraft:stone']]] if index%3==0 else []
        f=row(state,delta,step=rng.choice([0,0.5,0.6,1.0,1.5]),grounded=bool(index%2),context=index%4,
              offset=offset,fail=rng.choice([-1,-1,-1,0,1,2,3,8,50,100,150,250]),extra=extra,entities=entities,border=border)
        f['entities']=[rng.randrange(3),rng.randrange(3)]
        yield f


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL/'rust-world-movement-parity';build.mkdir(exist_ok=True)
    report_path = build/'report.json';report_path.unlink(missing_ok=True)
    oracle = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaRegistryCollisionScanOracle.java'
    helpers = [oracle.with_name(name+'.java') for name in
               ['VanillaHeightmapOracle', 'VanillaCollisionOracle', 'VanillaBlockStateCodecOracle', 'VanillaSuffocationOracle', 'VanillaFaceSupportOracle']]
    sources = [oracle, *helpers, Path(__file__), lab.ROOT/'tools/test_rust_parity.py',
               lab.ROOT/'tools/test_worldgen_parity.py', lab.ROOT/'tools/lab.py',
               lab.ROOT/'Cargo.toml', lab.ROOT/'Cargo.lock',
               lab.ROOT/'crates/horizon-world/examples/probe/registry_collision_scan_fixture.rs']
    for crate in ['horizon-core', 'horizon-world']:
        sources += [lab.ROOT/'crates'/crate/'Cargo.toml',
                    *sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested = base.source_hashes(sources)
    tags = worldgen.tag_resources(jars['server'])
    if tags != worldgen.tag_resources(jars['client']):raise AssertionError('Client/server block tags differ')
    lab.write_json(build/'tags.json', tags)
    classes = build/'classes';classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, libraries))
    def run(command, name):
        with (build/(name+'.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600,
                           stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp',
         str(jars['server'])+os.pathsep+dependencies, '-d', classes, oracle, *helpers], 'compile-java')
    def java(side, mode):
        run(['java', '-Xmx768M', '-cp', os.pathsep.join([str(classes), str(jars[side]), dependencies]),
             'dev.horizonlab.parity.VanillaRegistryCollisionScanOracle', mode, build], side+'-'+mode)
    java('server', 'export')
    facts = json.loads((build/'facts.json').read_text())
    if len(facts['states']) != 35723:raise AssertionError('Incomplete pinned registry')
    definitions = json.loads((build/'definitions.json').read_text())
    count = 0
    with (build/'inputs.txt').open('w') as output:
        for fixture in corpus(facts):
            output.write(json.dumps(fixture, separators=(',', ':'))+'\n');count += 1
    fixture_paths = [build/name for name in ['inputs.txt', 'facts.json', 'definitions.json',
                                            'block-registry.nbt', 'cached-collision.nbt', 'tags.json']]
    fixtures = base.source_hashes(fixture_paths)
    run(['cargo', 'build', '--locked', '--release', '-p', 'horizon-world', '--example', 'world_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary, 'registry_collision_scan_fixture', build], 'rust');references = []
    for side, jar in jars.items():
        java(side, side)
        matched = base.compare(build/(side+'.txt'), build/'rust.txt')
        references.append({'side':side, 'jar_sha256':lab.digest(jar, 'sha256'),
                           'output_sha256':lab.digest(build/(side+'.txt'), 'sha256'), 'matched_output_lines':matched})
        print(f'{side}: {matched:,} world movement lines match', flush=True)
    base.ensure_sources_unchanged(sources, tested);base.ensure_sources_unchanged(fixture_paths, fixtures)
    report = {'schema':1, 'minecraft':lock['minecraft']['version'], 'manifest_sha1':lock['minecraft']['manifest_sha1'],
              'platform':platform.platform(), 'java':subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc':subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'registry_states':len(facts['states']), 'block_definitions':len(definitions),
              'definition_counts':dict(collections.Counter(definitions.values())),
              'native_chunk_storage':'ProtoChunk', 'rust_chunk_storage':'TerrainChunk', 'fixtures':count, 'sources_sha256':tested, 'fixtures_sha256':fixtures,
              'rust_output_sha256':lab.digest(build/'rust.txt', 'sha256'), 'references':references,
              'scope':'Actual native Entity.collide executes initial collision and step-up decisions over BlockCollisions, registered block shapes and resident ProtoChunk sections. Rust combines owned/fallible step resolution with registry-bound TerrainChunk scanning. All registered states are used as obstacles, plus step heights, falling/grounded motion, zero and underflow motion, partial blocks including exact falling-block-sized boxes over slabs, stairs, snow, fences, iron bars, soul sand and farmland, fluids, dynamic blocks, ceilings, chunk boundaries and query failures. Entity collider lists and world-border proximity/shape are controlled inputs; query order and reuse, shape collection and resulting displacement are compared. Contexts are deliberately supplied empty/position/placement snapshots in both worlds. This does not establish entity broad-phase selection, actual world-border proximity, travel/gravity/velocity or grounded side effects, experimental minecarts or a playable game.'}
    lab.write_json(report_path, report);print(f'Scoped world movement parity passed: {report_path}', flush=True)


if __name__ == '__main__':main()
