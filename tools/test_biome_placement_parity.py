#!/usr/bin/env python3
"""Compare biome placement filtering, root context, nested placers and partial failures."""
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


def corpus():
    biome={'type':'biome'}
    chains=[[biome],[{'type':'count','count':3},{'type':'in_square'},biome],[biome,{'type':'count','count':2},{'type':'offset','x':1,'y':0,'z':0},biome],[{'type':'random_chance','chance':0.5},biome,{'type':'count','count':2}],[{'type':'count','count':2},biome,{'type':'offset','x':1,'y':0,'z':0},biome]]
    for legacy in [False,True]:
        for worldgen in [False,True]:
            for top in [False,True]:
                for membership in [[0,0],[1,0],[0,1],[1,1],[2,3],[3,2]]:
                    for placement in chains:
                        for query_fail,feature_fail in [(-1,-1),(0,-1),(1,-1),(3,-1),(-1,0),(-1,2)]:
                            for nested in [False,True]:
                                for return_mode in [0,1,2]:
                                    yield {'legacy':legacy,'worldgen':worldgen,'seed':-1 if nested else 0,'top':top,'direct':False,'membership':membership,'placement':placement,'query_fail':query_fail,'feature_fail':feature_fail,'nested':nested,'draw':return_mode!=1,'return_mode':return_mode,'origin':[15,-64,15]}
    for legacy in [False,True]:
        for top in [False,True]:
            for membership in itertools.product(range(4),repeat=2):
                for query_fail in [-1,0,1,2]:
                    for origin in [[0,0,0],[2147483647,-2147483648,2147483647]]:
                        yield {'legacy':legacy,'worldgen':False,'seed':0,'top':top,'direct':True,'membership':list(membership),'placement':[biome],'query_fail':query_fail,'feature_fail':-1,'nested':False,'draw':True,'return_mode':0,'origin':origin}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-biome-placement-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBiomePlacementOracle.java'
    helper=oracle.with_name("VanillaPlacementOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/biome_placement_fixture.rs']
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'biome_placement_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaBiomePlacementOracle',side,build],side)
        reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} biome-placement output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'random_modes':2,'requested_placements':len(fixtures)*3,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BiomeFilter.modify and FeaturePlacer.place/placeWithBiomeCheck with native biome generation settings. Missing top features fail before biome lookup. Direct, absent, structurally equal and different placed-feature membership, query order, callback random draws/results/errors, dirty buffers across repeats and nested plain placers are compared. Direct and WorldgenRandom-wrapped legacy/xoroshiro streams execute modifier chains. Rust delegates biome resolution and generation-settings membership to the caller while isolating each placer invocation context. Full biome sampling/registry equality, complete decoration, crash-report text and gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped biome-placement parity passed: {report_path}')


if __name__=='__main__':main()
