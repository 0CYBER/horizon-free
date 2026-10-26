#!/usr/bin/env python3
"""Compare registry-bound collision scans over resident chunks with pinned vanilla."""
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
    def row(state, context=0, suffocating=False, at=(0,0,0), bounds=None, entities=(0,0), fail=-1, limit=100000):
        x,y,z=at
        chunks=[[cx,cz] for cx in range((x>>4)-1,(x>>4)+2) for cz in range((z>>4)-1,(z>>4)+2)]
        if bounds is None:bounds=[x+0.2,y+0.2,z+0.2,x+0.8,y+0.8,z+0.8]
        return dict(chunks=chunks,blocks=[[*at,state]],context=context,suffocating=suffocating,
                    box=list(map(bits,bounds)),y=bits(y+1.0),entities=entities,fail=fail,limit=limit)
    states=facts['states']
    for state in range(len(states)):
        for suffocating in [False,True]:yield row(state,state%4,suffocating)
    dynamic=[i for i,s in enumerate(states) if s['id'].endswith('shulker_box') or s['id'] in
             ['minecraft:scaffolding','minecraft:water','minecraft:lava','minecraft:powder_snow',
              'minecraft:moving_piston','minecraft:bamboo','minecraft:pointed_dripstone','minecraft:sulfur_spike',
              'minecraft:piston','minecraft:sticky_piston','minecraft:oak_stairs','minecraft:stone_slab']]
    for state in dynamic:
        for context in range(4):
            for suffocating in [False,True]:
                for at in [(15,0,15),(-16,-1,-16)]:yield row(state,context,suffocating,at=at)
        if states[state]['id'].endswith('shulker_box'):
            for global_mode in range(3):
                for chunk_mode in range(3):
                    for suffocating in [False,True]:
                        for fail in [-1,0,1,8,15,20,24]:
                            yield row(state,suffocating=suffocating,entities=(global_mode,chunk_mode),fail=fail)
    rng=random.Random(0x5245534944454e54)
    for index in range(1500):
        at=rng.choice([(15,0,15),(-16,0,-16),(0,-16,0),(0,31,0)])
        x,y,z=at
        f=row(rng.randrange(len(states)),context=index%4,suffocating=bool(index%2),at=at,
              bounds=[x-0.2,y-0.2,z-0.2,x+1.2,y+1.2,z+1.2],
              entities=(rng.randrange(3),rng.randrange(3)),fail=rng.choice([-1,-1,0,5,20,50]),limit=rng.choice([0,1,3,100000]))
        f['chunks']=[c for c in f['chunks'] if c==[x>>4,z>>4] or rng.randrange(2)]
        for _ in range(12):
            px,pz=x+rng.randrange(-1,2),z+rng.randrange(-1,2)
            py=min(31,max(-16,y+rng.randrange(-1,2)))
            if [px>>4,pz>>4] in f['chunks']:f['blocks'].append([px,py,pz,rng.choice(dynamic)])
        yield f


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL/'rust-registry-collision-scan-parity';build.mkdir(exist_ok=True)
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
        print(f'{side}: {matched:,} resident collision-scan lines match', flush=True)
    base.ensure_sources_unchanged(sources, tested);base.ensure_sources_unchanged(fixture_paths, fixtures)
    report = {'schema':1, 'minecraft':lock['minecraft']['version'], 'manifest_sha1':lock['minecraft']['manifest_sha1'],
              'platform':platform.platform(), 'java':subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc':subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'registry_states':len(facts['states']), 'block_definitions':len(definitions),
              'definition_counts':dict(collections.Counter(definitions.values())),
              'native_chunk_storage':'ProtoChunk', 'rust_chunk_storage':'TerrainChunk', 'fixtures':count, 'sources_sha256':tested, 'fixtures_sha256':fixtures,
              'rust_output_sha256':lab.digest(build/'rust.txt', 'sha256'), 'references':references,
              'scope':'Native BlockCollisions over actual registered block behavior and resident ProtoChunk sections compared with RegistryCollisionWorld, contextual dispatch and TerrainChunk storage. All states cover ordinary and suffocating-only selection. Context variants include empty, always-fluid, position and placement; dynamic and representative partial shapes cover positive/negative chunk boundaries. Shulker cases distinguish chunk-local suffocation entities from global collision entities, with ordered failure injection. Seeded scenes cover resident/missing chunks, build-height edges, block replacements and early consumer termination. This validates registry/storage integration for these snapshots, not chunk loading, entity lifecycle, experimental minecarts or gameplay movement.'}
    lab.write_json(report_path, report);print(f'Scoped registry collision-scan parity passed: {report_path}', flush=True)


if __name__ == '__main__':main()
