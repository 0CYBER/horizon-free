#!/usr/bin/env python3
"""Compare native bundle weights and strict scalar stack validation."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
import math
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    edges=[-2147483648,-2147483647,-65537,-64,-2,-1,0,1,2,3,16,64,65537,2147483646,2147483647]
    rng=random.Random(0x42554e444c4557)
    for a in edges:
        for b in edges:
            for c,d in [(a,b),(1,16),(-a if a!=-2147483648 else a,1),(2147483647,2147483646)]:
                yield dict(mode='fraction',a=a,b=b,c=c,d=d)
    for _ in range(8000):
        def integer():return rng.choice(edges) if rng.randrange(2) else rng.randrange(-2147483648,2147483648)
        yield dict(mode='fraction',a=integer(),b=integer(),c=integer(),d=integer())
    def item(count=1,maximum=None,kind=0,bees=False,nested=-1):return dict(count=count,maximum=maximum,kind=kind,bees=bees,nested=nested)
    def weight(i):return dict(kind='weight',index=i)
    def bind(i,m):return dict(kind='prototype',index=i,maximum=m)
    def valid(count=1,maximum=1,damage=False,container=-1,bundle=-1,projectiles=-1):return dict(kind='validate',count=count,maximum=maximum,damage=damage,container=container,bundle=bundle,projectiles=projectiles)
    for count in [n for n in edges if n]:
        for maximum in [None,*edges]:
            for bees in [False,True]:
                for nested in [-1,0]:
                    yield dict(mode='bundle',prototypes=[64,1],bundles=[[item(count,maximum,bees=bees)],[item(1,maximum,bees=bees,nested=nested)]],ops=[valid(bundle=1),bind(0,2),weight(1),bind(0,0),weight(0),weight(1)])
    for _ in range(3000):
        bundles=[]
        for i in range(rng.randrange(1,7)):
            bundles.append([item(rng.choice([n for n in edges if n]),rng.choice([None,*edges]),rng.randrange(2),bool(rng.randrange(2)),rng.randrange(-1,i)) for _ in range(rng.randrange(5))])
        ops=[]
        for __ in range(12):
            def index():return rng.randrange(-1,len(bundles))
            ops.extend([valid(rng.choice(edges),rng.choice([None,*edges]),bool(rng.randrange(2)),index(),index(),index()),bind(rng.randrange(2),rng.choice([None,*edges])),weight(rng.randrange(len(bundles)))])
        yield dict(mode='bundle',prototypes=[rng.choice([None,*edges]),rng.choice([None,*edges])],bundles=bundles,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-bundle-weight-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBundleWeightOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/bundle_weight_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaBundleWeightOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'bundle_weight_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} bundle weight output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual BundleContents.weight with ItemStackTemplate contents, nested bundles, bee presence, changed holder prototypes and memoized success/error results. Direct pinned Apache Fraction construction/add/multiply probes verify intermediate checked arithmetic, signs, reduction and overflow. Native ItemStack.validateStrict additionally checks container sizes, bundle sizes and weight, charged projectile sizes, empty visibility and top-level scalar constraints; follow-up weight probes expose lazy validation order. Rust typed component/schema adapters remain fixture-local; full bundle inventory operations, codecs, item-use integration and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped bundle weight parity passed: {report_path}')


if __name__=='__main__':main()
