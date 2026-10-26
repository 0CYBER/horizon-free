#!/usr/bin/env python3
"""Compare registered movement properties, support positions and block factors with pinned vanilla."""
import collections
import json
import os
from pathlib import Path
import platform
import subprocess
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(info):
    import struct
    def bits(value):return struct.pack('>d',value).hex()
    defaults=info['defaults'];stone=defaults['minecraft:stone'];honey=defaults['minecraft:honey_block'];soul=defaults['minecraft:soul_sand']
    def row(state,mode,tags=0,support=[17,-64,-33],fail=-1,others=None):return dict(states=[state,*(others or [honey,soul])],mode=mode,tags=tags,position=list(map(bits,[17.3,1.5,-33.3])),cached=[17,1,-34],support=support,offset='3f000000',fail=fail)
    for tag in range(3):
        for state in range(info['states']):
            for mode in [0,1,2,3,4,5]:yield row(state,mode,tag)
        for name in ['stone','water','bubble_column','soul_sand','honey_block','slime_block','oak_fence','cobblestone_wall','oak_fence_gate','ice','packed_ice','blue_ice','bedrock','white_bed']:
            for mode in range(6):
                for support in [None,[17,-64,-33]]:
                    for fail in range(-1,4):
                        for first in [stone,defaults['minecraft:'+name]]:
                            yield row(first,mode,tag,support,fail,[defaults['minecraft:'+name],soul])


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL/'rust-movement-registry-parity';build.mkdir(exist_ok=True)
    report_path = build/'report.json';report_path.unlink(missing_ok=True)
    oracle = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMovementRegistryOracle.java'
    helpers = [oracle.with_name(name+'.java') for name in
               ['VanillaHeightmapOracle']]
    sources = [oracle, *helpers, Path(__file__), lab.ROOT/'tools/test_rust_parity.py',
               lab.ROOT/'tools/test_worldgen_parity.py', lab.ROOT/'tools/lab.py',
               lab.ROOT/'Cargo.toml', lab.ROOT/'Cargo.lock',
               lab.ROOT/'crates/horizon-world/examples/probe/movement_registry_fixture.rs']
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
             'dev.horizonlab.parity.VanillaMovementRegistryOracle', mode, build], side+'-'+mode)
    java('server', 'export')
    facts = json.loads((build/'registry-info.json').read_text())
    if facts['states'] != 35723:raise AssertionError('Incomplete pinned registry')
    count = 0
    with (build/'inputs.txt').open('w') as output:
        for fixture in corpus(facts):
            output.write(json.dumps(fixture, separators=(',', ':'))+'\n');count += 1
    fixture_paths = [build/name for name in ['inputs.txt', 'registry-info.json', 'movement-properties.nbt',
                                            'block-registry.nbt', 'tags.json']]
    fixtures = base.source_hashes(fixture_paths)
    run(['cargo', 'build', '--locked', '--release', '-p', 'horizon-world', '--example', 'world_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary, 'movement_registry_fixture', build], 'rust');references = []
    for side, jar in jars.items():
        java(side, side)
        matched = base.compare(build/(side+'.txt'), build/'rust.txt')
        references.append({'side':side, 'jar_sha256':lab.digest(jar, 'sha256'),
                           'output_sha256':lab.digest(build/(side+'.txt'), 'sha256'), 'matched_output_lines':matched})
        print(f'{side}: {matched:,} registered movement lines match', flush=True)
    base.ensure_sources_unchanged(sources, tested);base.ensure_sources_unchanged(fixture_paths, fixtures)
    report = dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),registry_states=facts['states'],registry_blocks=len(facts['defaults']),fixtures=count,sources_sha256=tested,fixtures_sha256=fixtures,rust_output_sha256=lab.digest(build/'rust.txt','sha256'),references=references,scope='Imports raw registered friction/speed/jump/bounce/fall-distance coefficients and fence-gate type; binds resolved fence/wall/bounce-suppression tags. Actual native getters independently verify every state. Entity support positions and jump/speed factors run every registered state with natural, empty and universal tags; additional mixed-block queries, null support and failures exercise lazy behavior. Does not establish live player travel, bounce effects or full gameplay.')
    lab.write_json(report_path,report);print(f'Scoped movement registry parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
