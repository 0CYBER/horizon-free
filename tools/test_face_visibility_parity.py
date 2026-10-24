#!/usr/bin/env python3
"""Compare neighboring block-face visibility against both pinned Java JARs."""
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def pairs(definitions):
    count = sum(len(v['states']) for v in definitions.values())
    common = [definitions['minecraft:'+name]['default'] for name in
              ['air','stone','glass','glass_pane','iron_bars','water','lava','oak_leaves',
               'mangrove_roots','powder_snow','stone_slab','oak_stairs']]
    rng = random.Random(0x4641434553)
    for value in definitions.values():
        states = value['states']
        for state in states:
            neighbors = [*common, state, value['default'], (state+1)%count, (state-1)%count,
                         *(rng.randrange(count) for _ in range(4))]
            for neighbor in dict.fromkeys(neighbors):
                yield state, neighbor
        # Exhaust all same-block connection/level/property combinations in every
        # family that overrides skipRendering, including waterlogged variants.
        if value['rule'] != 'never':
            for a in states:
                for b in states:
                    yield a, b
    # Cross-family bars connections, differing materials and tag membership.
    bars = [v for v in definitions.values() if v['rule'] == 'bars']
    for a in bars:
        for b in bars:
            for left in a['states']:
                for right in b['states']:
                    yield left, right


def main():
    lock,jars,libraries = base.verify_artifacts()
    build = lab.LOCAL/'rust-face-visibility-parity';build.mkdir(exist_ok=True)
    report_path = build/'report.json';report_path.unlink(missing_ok=True)
    oracle = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFaceVisibilityOracle.java'
    helpers = [oracle.with_name(name) for name in ['VanillaCollisionOracle.java','VanillaHeightmapOracle.java']]
    sources = [oracle,*helpers,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_worldgen_parity.py',lab.ROOT/'tools/lab.py',
               lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/examples/probe/face_visibility_fixture.rs']
    for crate in ['horizon-core','horizon-world']:
        sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested = base.source_hashes(sources)
    tags = worldgen.tag_resources(jars['server'])
    if tags != worldgen.tag_resources(jars['client']): raise AssertionError('Client/server block tags differ')
    lab.write_json(build/'tags.json',tags)
    classes = build/'classes';classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:
            subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,oracle,*helpers],'compile-java')
    def java(side,mode):
        run(['java','-Xmx768M','-cp',str(classes)+os.pathsep+str(jars[side])+os.pathsep+dependencies,
             'dev.horizonlab.parity.VanillaFaceVisibilityOracle',mode,build],side+'-'+mode)
    java('server','export')
    definitions = json.loads((build/'definitions.json').read_text());pair_count=0
    with (build/'pairs.txt').open('w') as output:
        for a,b in pairs(definitions):output.write(f'{a} {b}\n');pair_count+=1
    fixture_paths = [build/name for name in ['pairs.txt','block-registry.nbt','occlusion.nbt','definitions.json','tags.json']]
    fixtures = base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT))
    binary = Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'face_visibility_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side)
        matched = base.compare(build/(side+'.txt'),build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} face visibility masks match',flush=True)
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report = {'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],
              'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],
              'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'registry_states':sum(len(v['states']) for v in definitions.values()),
              'block_definitions':len(definitions),'behavior_kinds':sorted({v['rule'] for v in definitions.values()}),
              'pairs':pair_count,'direction_and_leaf_setting_cases':pair_count*12,'sources_sha256':tested,'fixtures_sha256':fixtures,
              'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,
              'scope':'Actual BlockState.skipRendering and Block.shouldRenderFace for all registered states against representative and seeded random neighbors, exhaustive same-block pairs for override families and every cross-material bars/panes connection state pair. All six directions and both cutout-leaf settings; pinned block tags bound on both Java sides. Rust imports raw occlusion grids, class-rule assignments, definition canOcclude flags and tag membership, then computes face geometry, canonical shortcuts, skip rules and ONLY_FIRST subtraction. Includes waterlogged fluid neighbors, glass identity, vertical roots, bars tag/property checks and partial occlusion. Does not establish chunk mesh integration, lighting, translucent sorting, fluid surfaces, GPU pixels or gameplay parity.'}
    lab.write_json(report_path,report);print(f'Scoped face visibility parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
