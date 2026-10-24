#!/usr/bin/env python3
"""Compare player ledge backoff through registered world collision probes with pinned vanilla."""
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
    def row(state,delta=(.8,-.08,.8),step=.6,grounded=True,context=0,offset=0,fail=-1,extra=(),entities=(),border=None,fall=0,staying=True,flying=False,mover='SELF'):
        blocks=[[offset,-1,0,state],*extra]
        return dict(chunks=[[x,z] for x in range((offset>>4)-1,(offset>>4)+2) for z in [-1,0,1]],blocks=blocks,context=context,y=bits(0),entities=[0,0],fail=fail,limit=100000,suffocating=False,box=list(map(bits,[offset+.2,0,.2,offset+.8,1.8,.8])),movement=dict(delta=list(map(bits,delta)),step=bits(step),grounded=grounded,entities=[list(map(bits,b)) for b in entities],border=None if border is None else list(map(bits,border)),edge=dict(staying=staying,flying=flying,mover=mover,fall=bits(fall))))
    states=facts['states'];defaults=facts['defaults']
    for state in range(len(states)):yield row(state,context=state%4)
    obstacles=[i for i,s in enumerate(states) if s['id'] in ['minecraft:oak_stairs','minecraft:stone_slab','minecraft:snow','minecraft:scaffolding','minecraft:powder_snow','minecraft:water','minecraft:shulker_box','minecraft:moving_piston','minecraft:piston','minecraft:soul_sand','minecraft:mud','minecraft:farmland','minecraft:bamboo','minecraft:pointed_dripstone']]
    for state in obstacles:
        for step in [0,.5,.6,1.0]:
            for fall in [0,.2,.6,1.0]:yield row(state,step=step,grounded=False,fall=fall,context=state%4)
    for mover in ['SELF','PLAYER','PISTON','SHULKER','SHULKER_BOX']:
        for flying in [False,True]:
            for staying in [False,True]:
                for y in [-.08,0,.1]:
                    for fail in [-1,0,1,3,10,30,100]:yield row(defaults['minecraft:stone'],delta=(.8,y,.8),mover=mover,flying=flying,staying=staying,fail=fail)
    rng=random.Random(0x574f524c44454447)
    for index in range(2500):
        offset=rng.choice([-16,0,15]);state=rng.choice(obstacles);delta=[rng.choice([-1,-.05,-0.0,0,.05,.5,.8,1]),rng.choice([-.5,-.08,0,.1]),rng.choice([-1,-.5,0,.5,.8,1])]
        entities=[] if index%3 else [[offset+.2,-1,.2,offset+.8,0,.8]]
        border=None if index%4 else [offset+1,-2,-2,offset+1.2,3,3]
        extra=[] if index%2 else [[offset+1,-1,1,defaults['minecraft:stone']]]
        f=row(state,delta,step=rng.choice([0,.5,.6,1]),grounded=bool(index%2),context=index%4,offset=offset,fail=rng.choice([-1,-1,0,3,15,50]),entities=entities,border=border,extra=extra,fall=rng.choice([0,.2,.6,1]));f['entities']=[rng.randrange(3),rng.randrange(3)];yield f


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL/'rust-world-edge-backoff-parity';build.mkdir(exist_ok=True)
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
        print(f'{side}: {matched:,} world edge-backoff lines match', flush=True)
    base.ensure_sources_unchanged(sources, tested);base.ensure_sources_unchanged(fixture_paths, fixtures)
    report = {'schema':1, 'minecraft':lock['minecraft']['version'], 'manifest_sha1':lock['minecraft']['manifest_sha1'],
              'platform':platform.platform(), 'java':subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc':subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'registry_states':len(facts['states']), 'block_definitions':len(definitions),
              'definition_counts':dict(collections.Counter(definitions.values())),
              'native_chunk_storage':'ProtoChunk', 'rust_chunk_storage':'TerrainChunk', 'fixtures':count, 'sources_sha256':tested, 'fixtures_sha256':fixtures,
              'rust_output_sha256':lab.digest(build/'rust.txt', 'sha256'), 'references':references,
              'scope':'Actual Player.maybeBackOffFromEdge issues native noCollision queries over registered block states in resident ProtoChunk storage. Rust combines edge-backoff guards/probe geometry with short-circuit entity-bearing no_world_collision and registry-bound TerrainChunk scanning. Every state is used as the supporting block; additional falling, step-height, chunk-edge, diagonal, partial/dynamic block, mover/flight/crouch and failing-provider cases are compared. Entity collider lists, border applicability/shapes and empty/position/placement contexts remain controlled inputs. This is registered-world ledge integration, not a complete Player.move/travel loop or full playable game.'}
    lab.write_json(report_path, report);print(f'Scoped world edge backoff parity passed: {report_path}', flush=True)


if __name__ == '__main__':main()
