#!/usr/bin/env python3
"""Compare the native world-generation random adapter, counters, forks and seeds."""
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
    rng=random.Random(78336)
    integers=[-2147483648,-10001,-1,0,1,16,214748,214749,2147483647]
    longs=[-9223372036854775808,-1,0,1,9223372036854775807]+[rng.randrange(-2**63,2**63) for _ in range(20)]
    sample=[['int'],['long'],['bool'],['float'],['double']]
    for legacy in [False,True]:
        for layers in [1,2,3]:
            for warmup in [0,7]:
                for seed in longs:
                    ops=[]
                    for bits in [-2147483648,-65,-64,-33,-1,0,1,24,26,27,31,32,33,48,63,64,65,2147483647]:ops.append(['bits',bits])
                    ops+=sample
                    for bound in [-2147483648,-1,0,1,2,3,7,16,1073741825,2147483647]:ops += [['bound',bound],*sample]
                    for x,z in zip(integers,reversed(integers)):
                        ops += [['decoration',seed,x,z],*sample,['feature',seed,z,x],*sample,['large',seed,x,z],*sample,['salt',seed,x,z,x],*sample,['slime',x,z,seed,seed^0x123456789abcdef]]
                    for operation in [['fork'],['pos',-2147483648,2147483647,-1],['pos_seed',seed],['pos_hash',[0,65,0xd800,0xdfff,0x20ac]]]:
                        ops += [['root'],operation,*sample,['fork'],*sample,['seed',seed],['consume',17],*sample]
                    ops += [['root'],['consume',-1],['consume',0],['consume',129],*sample]
                    yield {'legacy':legacy,'layers':layers,'warmup':warmup,'seed':seed,'ops':ops}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-worldgen-random-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaWorldgenRandomOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/worldgen_random_fixture.rs']
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'worldgen_random_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaWorldgenRandomOracle',side,build],side)
        reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} worldgen-random output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'random_modes':2,'wrapper_depths':[1,2,3],'operations':sum(len(f['ops']) for f in fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual WorldgenRandom bit sampling over legacy and xoroshiro sources, nested wrappers, signed bit-width shifts, draw counters, valid/invalid bounds, reseeding, fork and positional-factory delegation. Decoration, per-feature, large-feature, salt and slime-chunk seeds exercise signed int/long overflow, extreme coordinates and inherited legacy multi-draw behavior. Gaussian samples, concurrent access and full chunk decoration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped worldgen-random parity passed: {report_path}')


if __name__=='__main__':main()
