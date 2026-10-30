#!/usr/bin/env python3
"""Compare the feature branch of native chunk biome decoration."""
import argparse
import itertools
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base, test_feature_sort_parity as sort
except ImportError:
    import lab
    import test_rust_parity as base
    import test_feature_sort_parity as sort


def fixture(biomes, seed=0, mode=0, present=None):
    count=max((v for b in biomes for s in b for v in s),default=-1)+1
    chains=[[],[{'type':'count','count':3},{'type':'in_square'}],[{'type':'count','count':3},{'type':'in_square'},{'type':'biome'}],[{'type':'biome'},{'type':'random_chance','chance':0.5},{'type':'count','count':2}]]
    if present is None:present=list(range(len(biomes)+1))
    if not present:present=[len(biomes)]
    return {'biomes':biomes,'placements':[chains[mode%len(chains)] for _ in range(count)],'palettes':[present[i%len(present):]+present[:i%len(present)] for i in range(9)],'seed':seed,'cx':0,'cz':0,'min':-64,'height':32,'biome_offset':0,'chunk_fail':-1,'notify_fail':-1,'query_fail':-1,'feature_fail':-1,'return_mode':0}


def corpus(resources):
    graphs=[[],[[]],[[[],[],[]]],[[[0,1,2]]],[[[0,2]],[[1,2]]],[[[0],[],[1,2]],[[1],[],[2,3]]],[[[0,0]]],[[[0,1]],[[1,0]]],[[[0]],[[1]]],[[[],[2]],[[],[0,1]]],[[[0],[],[0]],[[1],[2],[3]]],[[[0]]+[[] for _ in range(12)]+[[1]],[[1]]]]
    policies=[{}, {'feature_fail':0},{'feature_fail':3},{'query_fail':0},{'query_fail':2},{'chunk_fail':0},{'chunk_fail':4},{'notify_fail':0},{'notify_fail':2},{'return_mode':1},{'return_mode':2}]
    for graph,mode,seed,policy in itertools.product(graphs,range(4),[0,-1,2**63-1],policies):
        f=fixture(graph,seed,mode);f.update(policy)
        f['cx']=[0,-1,1874999,134217728][mode];f['cz']=[0,-1875000,1,-134217729][mode]
        f['min']=[-64,0,-2032,1968][mode];f['height']=[16,32,64,16][mode];f['biome_offset']=mode
        yield f
    # Resident palettes can contain only an excluded biome, only a neighbor's
    # feature, or duplicates. Selection must retain global ordering and indices.
    graph=[[[0,3]],[[1,3]],[[2,3]]]
    for present in [[],[0],[1],[2],[0,2],[3],[0,0,3],[2,1,0]]:
        for mode in range(4):
            f=fixture(graph,-9223372036854775808,mode,present)
            f['palettes']=[([3] if i!=8 else present or [3]) for i in range(9)]
            yield f
    names=sorted({name for steps in resources.values() for step in steps for name in step})
    if any(not isinstance(name,str) or name.startswith('#') for name in names):raise ValueError('unsupported fixture holder form')
    identity={name:i for i,name in enumerate(names)}
    biomes=[[[identity[name] for name in step] for step in steps] for steps in resources.values()]
    rng=random.Random(8845)
    for mode in range(4):
        for selected in [list(range(len(biomes))),[],*[ [i] for i in range(len(biomes))],*[rng.sample(range(len(biomes)),rng.randrange(len(biomes)+1)) for _ in range(24)]]:
            f=fixture(biomes,rng.randrange(-2**63,2**63),mode,selected)
            f['cx']=rng.randrange(-1875000,1875000);f['cz']=rng.randrange(-1875000,1875000)
            yield f
    for _ in range(200):
        # Shared order avoids cycles while exercising sparse high step indices.
        biomes=[[[i for i in range(12) if rng.randrange(5)==0] for _ in range(rng.randrange(15))] for _ in range(rng.randrange(8))]
        f=fixture(biomes,rng.randrange(-2**63,2**63),rng.randrange(4));f.update(rng.choice(policies));yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-decoration-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    resources=sort.biome_features(jars['server'])
    if len(resources)!=67 or resources!=sort.biome_features(jars['client']):raise ValueError('pinned biome feature resources differ')
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDecorationOracle.java';helper=oracle.with_name('VanillaPlacementOracle.java')
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_feature_sort_parity.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/decoration_fixture.rs']
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus(resources));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    resource_path=build/'biome-features.json';resource_path.write_text(json.dumps(resources,separators=(',',':')),encoding='utf-8')
    fixture_paths=[inputs,resource_path];tested_fixtures=base.source_hashes(fixture_paths)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'decoration_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaDecorationOracle',side,build],side)
        reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} decoration output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'biomes':len(resources),'requested_decorations':len(fixtures)*2,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual ChunkGenerator.applyBiomeDecoration with structure generation disabled, native ProtoChunk biome palettes, native FeatureSorter and WorldgenRandom, and traced feature sinks. Nine-chunk query order, possible-biome intersection, sorted global feature indices, decoration/feature seeds, biome-check placement, repeated calls, ignored false results, progress markers and partial failures are compared. All 67 bundled biome feature lists participate with synthetic payloads. Rust owns immutable generation settings and delegates resident palette transport and feature callbacks. Structure decoration, complete registry/world region integration, all feature algorithms, crash-report formatting and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped decoration parity passed: {report_path}')


if __name__=='__main__':main()
