#!/usr/bin/env python3
"""Compare registry collision flags and suffocation predicates with pinned vanilla."""
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


def corpus(facts):
    positions = [[0, 0, 0], [17, -64, -33],
                 [2147483647, -2147483648, -2147483648],
                 [-2147483648, 2147483647, 2147483647]]
    # Keep each tag binding contiguous to avoid testing registry reload costs.
    for tags in range(3):
        for state, value in enumerate(facts['states']):
            for position in positions:
                yield dict(state=state, position=position, entity='none', fail=-1, tags=tags)
            # Predicate short circuits must suppress failure-producing providers.
            yield dict(state=state, position=positions[1], entity='none', fail=0, tags=tags)
            if value['id'].endswith('shulker_box'):
                for position in positions:
                    for entity in ['none', 'other', 'closed', 'opening', 'opened', 'closing']:
                        for fail in [-1, 0, 1]:
                            yield dict(state=state, position=position, entity=entity, fail=fail, tags=tags)


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL/'rust-suffocation-parity';build.mkdir(exist_ok=True)
    report_path = build/'report.json';report_path.unlink(missing_ok=True)
    oracle = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSuffocationOracle.java'
    helpers = [oracle.with_name(name+'.java') for name in
               ['VanillaHeightmapOracle', 'VanillaCollisionOracle', 'VanillaBlockStateCodecOracle']]
    sources = [oracle, *helpers, Path(__file__), lab.ROOT/'tools/test_rust_parity.py',
               lab.ROOT/'tools/test_worldgen_parity.py', lab.ROOT/'tools/lab.py',
               lab.ROOT/'Cargo.toml', lab.ROOT/'Cargo.lock',
               lab.ROOT/'crates/horizon-world/examples/probe/suffocation_fixture.rs']
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
             'dev.horizonlab.parity.VanillaSuffocationOracle', mode, build], side+'-'+mode)
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
    run([binary, 'suffocation_fixture', build], 'rust');references = []
    for side, jar in jars.items():
        java(side, side)
        matched = base.compare(build/(side+'.txt'), build/'rust.txt')
        references.append({'side':side, 'jar_sha256':lab.digest(jar, 'sha256'),
                           'output_sha256':lab.digest(build/(side+'.txt'), 'sha256'), 'matched_output_lines':matched})
        print(f'{side}: {matched:,} suffocation and collision-metadata lines match', flush=True)
    base.ensure_sources_unchanged(sources, tested);base.ensure_sources_unchanged(fixture_paths, fixtures)
    report = {'schema':1, 'minecraft':lock['minecraft']['version'], 'manifest_sha1':lock['minecraft']['manifest_sha1'],
              'platform':platform.platform(), 'java':subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc':subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'registry_states':len(facts['states']), 'block_definitions':len(definitions),
              'definition_counts':dict(collections.Counter(definitions.values())),
              'tag_variants':3, 'positions':4, 'fixtures':count, 'sources_sha256':tested, 'fixtures_sha256':fixtures,
              'rust_output_sha256':lab.digest(build/'rust.txt', 'sha256'), 'references':references,
              'scope':'Actual BlockState.isSuffocating, hasLargeCollisionShape and moving-piston identity for every registered state, natural/empty/all-block causes_suffocation tags and four positions including integer extremes. Native predicate identity and constant overrides are independently classified. Rust computes cached fullness, dynamic collision fullness, piston extension and lazy shulker state queries. Every shulker state covers absent/wrong block entities, all animation states and injected query failures; all other states test provider short circuits. This does not establish live chunk/provider integration, entity movement, damage or full gameplay parity.'}
    lab.write_json(report_path, report);print(f'Scoped suffocation parity passed: {report_path}', flush=True)


if __name__ == '__main__':main()
