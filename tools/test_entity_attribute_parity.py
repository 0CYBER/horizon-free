#!/usr/bin/env python3
"""Compare registered entity defaults and lazy attribute lifecycle with vanilla."""
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
    def bits(v):return struct.pack('>d',float(v)).hex()
    def op(mode,attribute,**kw):
        value=dict(mode=mode,attribute=attribute,id='horizon:a',amount=bits(.125),operation=0,base=bits(7));value.update(kw);return value
    n=len(catalog['attributes'])
    values=[-float('inf'),-1e30,-1.,-0.,0.,1e-40,.1,1.,2.,1e16,1e30,float('inf'),float('nan')]
    for entity in catalog['entities']:
        for i in range(n):
            ops=[op(mode,i) for mode in ['has','base','value','modifier','has_modifier','reset','ensure','modifier','ack_update','ack_sync','value','set','base','value','reset','value','ack_update','ack_sync','add','value','modifier','has_modifier','add','permanent','update','value','remove','value','permanent','value','reset','value','clear','value','ack_sync','ack_update']]
            for operation in range(3):ops.extend([op('update',i,operation=operation),op('value',i)])
            yield dict(entity=entity,ops=ops)
    rng=random.Random(0x454e54415454)
    entities=list(catalog['entities'])
    for _ in range(1000):
        entity=rng.choice(entities);ops=[]
        for __ in range(rng.randrange(20,160)):
            ops.append(op(rng.choice(['has','base','value','modifier','has_modifier','reset','ensure','ack_update','ack_sync','set','add','update','permanent','remove','clear']),rng.randrange(n),id=rng.choice(['horizon:a','horizon:b','horizon:c']),amount=bits(rng.choice(values)),base=bits(rng.choice(values)),operation=rng.randrange(3)))
        yield dict(entity=entity,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-entity-attribute-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAttributeOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/entity_attribute_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEntityAttributeOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'catalog.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'catalog.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'entity_attribute_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} entity attribute output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Every registered attribute and entity DefaultAttributes supplier, actual AttributeMap per-attribute queries, lazy creation, mutation, base reset, update/sync membership and independent acknowledgements. Composes the verified attribute arithmetic/cache/modifier engine. Cross-attribute collection order is normalized to registry order; bulk assignment, custom suppliers with modifiers, save/network codecs, live entities and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped entity attribute parity passed: {report_path}')


if __name__=='__main__':main()
