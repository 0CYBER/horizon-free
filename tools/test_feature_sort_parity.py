#!/usr/bin/env python3
"""Compare identity-based feature ordering and cycles across biome steps."""
import argparse
import itertools
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def biome_features(jar):
    with zipfile.ZipFile(jar) as archive:
        return {name:json.loads(archive.read(name))['features'] for name in sorted(archive.namelist()) if name.startswith('data/minecraft/worldgen/biome/') and name.endswith('.json')}


def corpus(resources):
    sequences=[list(p) for size in range(4) for p in itertools.product(range(3),repeat=size)]
    for a,b in itertools.product(sequences,repeat=2):
        yield {'biomes':[[a],[b]],'queries':[0,1,2,3]}
        yield {'biomes':[[a,[],b],[b,[],a]],'queries':[0,1,2,3]}
    yield {'biomes':[],'queries':[0]}
    yield {'biomes':[[],[[],[],[]]],'queries':[0]}
    rng=random.Random(6628)
    for _ in range(500):
        biomes=[[[rng.randrange(12) for _ in range(rng.randrange(6))] for _ in range(rng.randrange(6))] for _ in range(rng.randrange(8))]
        yield {'biomes':biomes,'queries':list(range(13))}
    names=sorted({name for steps in resources.values() for step in steps for name in step})
    if any(not isinstance(name,str) or name.startswith('#') for name in names):raise ValueError('unsupported fixture holder form')
    identity={name:i for i,name in enumerate(names)}
    biomes=[[[identity[name] for name in step] for step in steps] for steps in resources.values()]
    queries=list(range(len(names)+1))
    yield {'biomes':biomes,'queries':queries}
    yield {'biomes':list(reversed(biomes)),'queries':queries}
    for biome in biomes:yield {'biomes':[biome],'queries':queries}
    for _ in range(100):
        sample=rng.sample(biomes,rng.randrange(len(biomes)+1))
        yield {'biomes':sample,'queries':queries}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-feature-sort-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    resources=biome_features(jars['server'])
    if len(resources)!=67 or resources!=biome_features(jars['client']):raise ValueError('pinned biome feature resources differ')
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFeatureSortOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/feature_sort_fixture.rs']
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus(resources));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    resource_path=build/'biome-features.json';resource_path.write_text(json.dumps(resources,separators=(',',':')),encoding='utf-8')
    fixture_paths=[inputs,resource_path];tested_fixtures=base.source_hashes(fixture_paths)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'feature_sort_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaFeatureSortOracle',side,build],side)
        reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} feature-sort output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'biomes':len(resources),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual FeatureSorter.buildFeaturesPerStep with structurally equal but identity-distinct placed-feature payloads. Ordered per-biome step lists, repeated references, cycles, cross-step reuse, disconnected nodes, empty steps and identity index lookups are compared. Every bundled biome feature list and deterministic shuffled/subset combinations bind named references to stable object identities without executing unported feature algorithms. Rust uses bounded iterative DFS. Cycle diagnostic minimization/text, full registry ownership, decoration seeding and feature execution remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped feature-sort parity passed: {report_path}')


if __name__=='__main__':main()
