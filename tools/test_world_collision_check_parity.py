#!/usr/bin/env python3
"""Compare short-circuit block/entity/border collision-free queries with pinned vanilla."""
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
    def row(state,bounds=(-.3,0,.2,.3,1.8,.8),context=0,entities=(),border=None,fail=-1,blocks=None):
        return dict(chunks=[[x,z] for x in [-1,0,1] for z in [-1,0,1]],blocks=blocks if blocks is not None else [[0,0,0,state]],context=context,y=bits(0),entities=[0,0],fail=fail,limit=100000,suffocating=False,box=list(map(bits,bounds)),movement=dict(check=True,delta=list(map(bits,[0,0,0])),step=bits(.6),grounded=True,entities=[list(map(bits,b)) for b in entities],border=None if border is None else list(map(bits,border))))
    for state in range(len(facts['states'])):
        yield row(state,context=state%4)
        yield row(state,bounds=(-.6,0,.2,-.2,1.8,.8),context=state%4,border=[-.5,-1,-1,-.4,3,1])
    defaults=facts['defaults'];names=['air','stone','water','lava','scaffolding','powder_snow','shulker_box','moving_piston','stone_slab','oak_stairs','snow','cobweb','bamboo']
    boxes=[[-.3,0,.2,.3,1.8,.8],[0,0,0,1,1,1],[0,0,0,0,0,0],[1,0,0,1.6,1.8,.6],[-16,-16,-16,-15,0,-15],[15.8,0,15.8,16.4,1.8,16.4]]
    entity_lists=[[],[[0,0,0,0,0,0]],[[0,0,0,1,1,1]],[[100,100,100,101,101,101]]]
    borders=[None,[-.5,-1,-1,-.4,3,1],[0,0,0,0,0,0],[100,100,100,101,101,101]]
    for name in names:
        for context in range(4):
            for entities in entity_lists:
                for border in borders:
                    for fail in [-1,0,1,3,12,25,50]:yield row(defaults['minecraft:'+name],context=context,entities=entities,border=border,fail=fail)
    rng=random.Random(0x434c454152)
    for _ in range(3000):
        state=rng.randrange(len(facts['states']));f=row(state,bounds=rng.choice(boxes),context=rng.randrange(4),entities=rng.choice(entity_lists),border=rng.choice(borders),fail=rng.choice([-1,-1,0,3,12,50]))
        f['entities']=[rng.randrange(3),rng.randrange(3)]
        if rng.randrange(3)==0:f['chunks']=[];f['blocks']=[]
        yield f


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL/'rust-world-collision-check-parity';build.mkdir(exist_ok=True)
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
        print(f'{side}: {matched:,} world collision-check lines match', flush=True)
    base.ensure_sources_unchanged(sources, tested);base.ensure_sources_unchanged(fixture_paths, fixtures)
    report = {'schema':1, 'minecraft':lock['minecraft']['version'], 'manifest_sha1':lock['minecraft']['manifest_sha1'],
              'platform':platform.platform(), 'java':subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc':subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'registry_states':len(facts['states']), 'block_definitions':len(definitions),
              'definition_counts':dict(collections.Counter(definitions.values())),
              'native_chunk_storage':'ProtoChunk', 'rust_chunk_storage':'TerrainChunk', 'fixtures':count, 'sources_sha256':tested, 'fixtures_sha256':fixtures,
              'rust_output_sha256':lab.digest(build/'rust.txt', 'sha256'), 'references':references,
              'scope':'Actual native CollisionGetter.noCollision for an entity executes lazy BlockCollisions over registered blocks and resident ProtoChunk sections, then entity list and applicable border shape checks. Rust uses the registry-bound TerrainChunk scan in the same short-circuit order. All registered states, controlled context variants, degenerate entity shapes, empty/disjoint borders, missing chunks, chunk/height boundaries and failures are compared. Entity list contents and border applicability/shapes are controlled; null-entity overloads, live entity indexing, experimental minecarts and player ledge behavior remain separate work.'}
    lab.write_json(report_path, report);print(f'Scoped world collision-check parity passed: {report_path}', flush=True)


if __name__ == '__main__':main()
