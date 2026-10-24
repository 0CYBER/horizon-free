#!/usr/bin/env python3
"""Compare registered item default components and use/durability query binding."""
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


def corpus(catalog):
    def bits(v):return struct.pack('>f',v).hex()
    def op(kind,value):return dict(kind=kind,value=value)
    names=[r['name'] for r in catalog['items']]
    for name in names:
        for count in [-1,0,1,2,64]:
            yield dict(name=name,count=count,ops=[op('set-damage',1),op('count',1),op('consume',bits(.75)),op('blocking',bits(.25)),op('kinetic',True),op('consume',None),op('blocking',None),op('kinetic',False),op('instrument',bits(1.25)),op('unbreakable',True),op('count',0),op('count',2)])
    floats=[-float('inf'),-1e30,-1,-.025,-0.,0.,1e-40,.025,.5,1,1.6,1e30,float('inf'),float('nan')]
    representatives=['minecraft:'+n for n in ['stone','apple','shield','wooden_spear','bow','crossbow','trident','bundle','brush','spyglass','ender_eye','goat_horn','air']]
    for name in representatives:
        for value in floats:
            for count in [0,1,2]:yield dict(name=name,count=count,ops=[op('consume',bits(value)),op('instrument',bits(value)),op('max-stack',2),op('max-damage',3),op('damage',0),op('damage',3),op('max-damage',None),op('max-stack',None)])
    rng=random.Random(0x4954454d434154)
    edges=[-2147483648,-2147483647,-1,0,1,2,20,2147483646,2147483647]
    def integer():return rng.choice(edges) if rng.randrange(2) else rng.randrange(-2147483648,2147483648)
    def random_op():
        kind=rng.choice(['count','set-damage','damage','max-damage','max-stack','unbreakable','consume','blocking','kinetic','instrument'])
        if kind in ['consume','blocking','instrument']:value=rng.choice([None,bits(rng.choice(floats)),f'{rng.getrandbits(32):08x}'])
        elif kind in ['unbreakable','kinetic']:value=bool(rng.randrange(2))
        elif kind in ['count','set-damage']:value=integer()
        else:value=rng.choice([None,integer()])
        return op(kind,value)
    for _ in range(10000):yield dict(name=rng.choice(representatives if rng.randrange(2) else names),count=rng.choice([-1,0,1,2,64]),ops=[random_op() for _ in range(rng.randrange(1,25))])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-use-catalog-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemUseCatalogOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_use_catalog_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemUseCatalogOracle',mode,path],side+'-'+mode)
    exports=[]
    for side in jars:
        folder=build/(side+'-export');folder.mkdir(exist_ok=True);java(side,'export',folder);exports.append(json.loads((folder/'catalog.json').read_text()))
    if exports[0]!=exports[1]:raise RuntimeError('native item catalogs differ')
    catalog_path=build/'catalog.json';lab.write_json(catalog_path,exports[0]);fixtures=list(corpus(exports[0]));inputs=build/'inputs.txt' ;inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,catalog_path];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_use_catalog_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item use catalog output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'registered_items':len(exports[0]['items']),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'All 1658 built-in item identities with bound native default component maps. Parsed exports from both JARs agree; Rust imports the component projection for stack size, durability and item-use duration/release behavior. Native ItemStack queries cover every default plus component/count mutations, empty/air behavior, registered method overrides, consumable truncation versus instrument flooring, integer and float boundaries. Other item components, arbitrary component codecs, full inventory/equipment, enchantment effects and gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item use catalog parity passed: {report_path}')


if __name__=='__main__':main()
