#!/usr/bin/env python3
"""Compare actual attribute map/supplier allocation, effect modifiers and live dirty queues."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    rng = random.Random(0x53544f5241545452)
    def bits(v): return struct.pack('>d', float(v)).hex()
    values = [-float('inf'), -1e30, -1, -0., 0., .1, 1, 2, 20, 1e16, float('inf'), float('nan')]
    def op(kind, **kw):
        r = dict(op=kind, key=0, instance=0, template=0, target=0, amplifier=0, flag=False, value=bits(0), fail=-1, mutate=-1, mutations=[])
        r.update(kw); return r
    def hashes(mode, n):
        def unmix(x):
            h = ((x ^ (x >> 16)) * pow(0x9e3779b9, -1, 2**32)) & 0xffffffff
            return h if h < 0x80000000 else h - 2**32
        if mode == 0: return [0] * n
        if mode == 1: return [unmix(31)] * n
        if mode == 2: return [unmix(127 + i % 4) for i in range(n)]
        if mode == 3: return list(range(n))
        return [rng.randrange(-2**31, 2**31) for _ in range(n)]
    def fixture(n, mode, ops, null=False):
        templates = [dict(id='horizon:m'+str(i), amount=bits(values[i % len(values)]), operation=i % 3) for i in range(n)]
        definitions = [op('define', key=i, template=i) for i in range(n)]
        if null: definitions.append(op('define', key=-1))
        return dict(hashes=hashes(mode,n), defaults=[bits(values[(i+6) % len(values)]) for i in range(n)],
                    bases=[None if i % 5 == 4 else bits(values[(i+8) % len(values)]) for i in range(n)],
                    syncable=[i % 2 == 0 for i in range(n)], templates=templates, definitions=definitions,
                    ops=[op('get')] + ops)
    for mode in range(5):
        for n in [1, 2, 7, 24, 25, 49, 97]:
            for null in [False, True]:
                ops = [op('get',key=i) for i in range(n)] + [op('get',key=i) for i in range(n)]
                ops += [op('add'),op('value'),op('queue-clear'),op('queue-clear',target=1),op('add',amplifier=2),op('value'),op('remove'),op('remove')]
                ops += [op('clear'),op('get',key=0),op('modified',instance=0),op('put',key=-1,instance=0),op('get',key=-1),op('add',amplifier=-1)]
                yield fixture(n,mode,ops,null)
    actions = [[op('put',instance=0)], [op('put',key=-1,instance=0)], [op('put',instance=-1)],
               [op('clear')], [op('queue-clear')], [op('queue-clear',target=1)],
               [op('queue-add',instance=-1)], [op('queue-add',target=1,instance=-1)],
               [op('queue-drop',instance=0)], [op('syncable',flag=False)], [op('syncable',flag=True)],
               [op('default',value=bits(float('nan')))], [op('default',value=bits(-0.))]]
    for n in [1,7,25]:
        for mode in [0,1,4]:
            for mutations in actions + [[op('put',key=i,instance=0) for i in range(n)]]:
                for at in [0,1,2,3]:
                    yield fixture(n,mode,[op('drop'),op('get',mutate=at,mutations=mutations),op('lookup'),op('get'),op('add'),op('remove')])
    for fail in range(8):
        for kind in ['get','add','remove','modified']:
            for sync in [False,True]:
                yield fixture(7,1,[op('syncable',flag=sync),op('drop'),op(kind,fail=fail),op('lookup'),op('get'),op('value'),op('add'),op('remove')])
    yield fixture(1,0,[op('modified',instance=-1),op('queue-clear'),op('put',instance=-1),op('lookup'),op('get'),op('get',key=-1)])
    for _ in range(200):
        n=rng.choice([1,7,24,25,49])
        def raw():
            return op(rng.choice(['put','drop','clear','queue-add','queue-drop','queue-clear','syncable','default']),
                      key=rng.randrange(n),instance=rng.choice([-1,0]),target=rng.randrange(2),flag=bool(rng.randrange(2)),value=bits(rng.choice(values)))
        ops=[]
        for __ in range(35):
            if rng.randrange(4)==0:ops.append(raw());continue
            ops.append(op(rng.choice(['get','get','add','remove','lookup','modified','value']),key=rng.randrange(-1,n),
                          amplifier=rng.choice([-2**31,-1,0,1,255,2**31-1]),instance=0,
                          fail=rng.choice([-1,-1,-1,0,1,2]),mutate=rng.choice([-1,-1,0,1,2,3]),
                          mutations=[raw() for ___ in range(rng.randrange(4))]))
        yield fixture(n,rng.randrange(5),ops,bool(rng.randrange(2)))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-stored-effect-attributes-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStoredEffectAttributesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/stored_effect_attributes_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaStoredEffectAttributesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';fixture_paths.append(enriched);tested_fixtures.update(base.source_hashes([enriched]))
        run([binary,'stored_effect_attributes_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';candidate=build/f'rust-{side}.txt';matched=base.compare(reference,candidate)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched,'rust_output_sha256':lab.digest(candidate,'sha256')})
        print(f'{side}: {matched:,} stored effect attribute output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'fastutil_sha256':{str(p.relative_to(lab.ROOT)):lab.digest(p,'sha256') for p in libraries if 'fastutil' in str(p)},'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Actual MobEffect modifier application bound to actual AttributeMap.getInstance, AttributeSupplier.createInstance and AttributeInstance replacement/modifier storage. Compares lazy creation, failed creation retained in update queues before map insertion, null cached entries, map key aliasing, native identity-hash update/sync set order, captured instance ownership after map clearing, dirty callback mutation/failure and value/cache/permanent state. Native instance identity hashes are exported per JVM for Rust replay. Supplier prototypes use real basic attributes and base values; isClientSyncable/getDefaultValue are controlled callbacks. Registered definitions, entity notification binding, bulk map methods and full gameplay/platform builds remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped stored effect attribute parity passed: {report_path}')


if __name__=='__main__':main()
