#!/usr/bin/env python3
"""Compare attribute modifier storage, value calculation and dirty callbacks with vanilla."""
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


def corpus(definitions):
    def bits(v):return struct.pack('>d',float(v)).hex()
    def definition(default=1,minimum=None,maximum=None):
        d=dict(default=bits(default))
        if minimum is not None:d.update(min=bits(minimum),max=bits(maximum))
        return d
    def mod(name='horizon:a',amount=.1,operation=0):return dict(id=name,amount=bits(amount),operation=operation)
    def op(mode='value',**kw):
        f=dict(mode=mode,target=0,source=1,modifier=0,modifiers=[],base=bits(0),fail=-1,read=False);f.update(kw);return f
    def row(modifiers,ops,defs=None):return dict(definitions=defs or [definition(),definition(0,-100,100)],modifiers=modifiers,ops=ops)
    basic=[mod(),mod(amount=.2,operation=1),mod(amount=.3,operation=2),mod(),mod('horizon:b',2,0)]
    sequence=[op('value'),op('permanent'),op('update',modifier=1),op('update',modifier=1),op('update',modifier=3),op('value'),op('remove'),op('value'),op('clear'),op('value'),op('apply',modifiers=[0,1,2,4]),op('pack'),op('unpack',target=1),op('copy',target=1,source=0),op('copy',target=0,source=0),op('value'),op('value',target=1)]
    for d in definitions:yield row(basic,sequence,defs=[d,definition()])
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,2,1e16,1e30,float('inf'),float('nan')]
    for value in values:
        for amount in values:
            for operation in range(3):yield row([mod(amount=amount,operation=operation)],[op('base',base=bits(value)),op('add'),op('value'),op('value'),op('remove'),op('value')])
    for default in values:
        for minimum in [-1,-0.,0.,1,float('nan')]:
            for maximum in [-1,-0.,0.,1,float('nan')]:yield row([],[op('value'),op('base',base=bits(default)),op('value')],defs=[definition(default,minimum,maximum),definition()])
    for fail in range(-1,5):
        for read in [False,True]:
            ops=[dict(o,fail=fail,read=read) for o in sequence]
            ops.extend([op('replace',modifier=4,fail=fail,read=read),op('many',modifiers=[0,4,0],fail=fail,read=read),op('value')])
            yield row(basic,ops)
    names=['a','minecraft:a',':a','','minecraft:',':','horizon:','horizon:/a','a:b.c-_/0']
    yield row([mod(n,i/10,i%3) for i,n in enumerate(names)],[*[op('update',modifier=i) for i in range(len(names))],op('value'),op('clear'),op('value')])
    def java_hash(s):
        h=0
        for c in s:h=(h*31+ord(c))&0xffffffff
        return h
    colliders=[];i=0
    while len(colliders)<128:
        name='cluster_'+str(i);h=(31*java_hash('horizon')+java_hash(name))&0xffffffff;h=(h*0x9e3779b9)&0xffffffff
        if (h^(h>>16))&31==31:colliders.append('horizon:'+name)
        i+=1
    for operation in range(3):
        pool=[mod(n,[1e16,-1e16,1,.01,-.01,7][i%6],operation) for i,n in enumerate(colliders)]
        for reverse in [False,True]:
            order=list(range(len(pool)));order=order[::-1] if reverse else order
            ops=[]
            for i in order:ops.extend([op('permanent',modifier=i),op('value')])
            ops.extend([op('copy',target=1,source=0),op('value',target=1)])
            for i in order[::2]+order[1::2]:ops.extend([op('remove',modifier=i),op('value')])
            ops.extend([op('apply',modifiers=order),op('value'),op('clear'),op('value')]);yield row(pool,ops)
    rng=random.Random(0x415454524942)
    for _ in range(1500):
        pool=[mod(rng.choice(colliders[:40]),rng.choice(values),rng.randrange(3)) for _ in range(rng.randrange(1,90))]
        ops=[op('pack')]
        for i in range(rng.randrange(10,100)):
            mode=rng.choice(['base','add','update','permanent','replace','remove','clear','copy','pack','unpack','apply','many','value'])
            ops.append(op(mode,target=rng.randrange(2),source=rng.randrange(2),modifier=rng.randrange(len(pool)),modifiers=[rng.randrange(len(pool)) for _ in range(rng.randrange(8))],base=bits(rng.choice(values)),fail=rng.choice([-1,-1,-1,0,1,2]),read=bool(rng.randrange(2))))
        yield row(pool,ops,defs=rng.sample(definitions,2) if rng.randrange(2) else [definition(),definition(0,-100,100)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-attribute-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaAttributeOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/attribute_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaAttributeOracle',mode,path],side+'-'+mode)
    java('server','export',build);states=json.loads((build/'attributes.json').read_text());fixtures=list(corpus(states));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'attributes.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'attribute_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} attribute output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual AttributeInstance and basic/ranged attributes, with every registered attribute definition. Native modifier identity, global/permanent insertion order, per-operation fastutil hash iteration/rehashing, cache/dirty callbacks, partial writes on observer failure, transient operation changes, duplicate rejection, bulk removal, replacement (including self), pack and merging apply. Includes nonfinite/signed-zero arithmetic, collision clusters and seeded operation sequences. Attribute registry/entity/equipment integration and packed data/network codecs remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped attribute parity passed: {report_path}')


if __name__=='__main__':main()
