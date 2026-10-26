#!/usr/bin/env python3
"""Compare native attribute map queries, cached and supplier values, modifier lookups and reset."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
try:
    from . import lab, test_rust_parity as base, test_stored_effect_attributes_parity as stored_base, test_stored_effect_notifications_parity as notifications
except ImportError:
    import lab
    import test_rust_parity as base
    import test_stored_effect_attributes_parity as stored_base
    import test_stored_effect_notifications_parity as notifications


def corpus():
    yield from notifications.corpus()
    rows=list(stored_base.corpus())
    def bits(v):return struct.pack('>d',float(v)).hex()
    values=[-float('inf'),-1e30,-1,-0.,0.,.1,1,2,20,1e16,float('inf'),float('nan')]
    def op(kind,**kw):
        r=dict(op=kind,key=0,instance=0,template=0,target=0,amplifier=0,flag=False,value=bits(0),fail=-1,mutate=-1,mutations=[],client=False,reapply=True,count=1)
        r.update(kw);return r
    def queries(key=0,template=0):
        return [op(k,key=key,template=template) for k in ['attribute-has','modifier-has','attribute-value','attribute-base','modifier-value','reset-base']]
    selected=[rows[i] for i in [0,4,8,12,18,22,36,50]]
    for row in selected:
        n=len(row['hashes'])
        ops=[]
        for key in [-1,*range(n)]:ops.extend(queries(key,min(max(key,0),n-1)))
        ops += [op('prototype-permanent'),*queries(),op('get'),*queries(),op('prototype-base',value=bits(7)),op('base',value=bits(3)),*queries()]
        ops += [op('prototype-remove'),*queries(),op('notify-add'),*queries(),op('notify-update',amplifier=2),*queries(),op('notify-remove'),*queries()]
        for key in [-1,*range(n)]:ops += [op('put',key=key,instance=0),*queries(key),op('put',key=key,instance=-1),*queries(key)]
        yield dict(row,ops=ops)
    for row in selected[:4]:
        for value in values:
            for base in values:
                yield dict(row,ops=[op('prototype-base',value=bits(value)),*queries(),op('prototype-permanent'),
                    op('get'),op('base',value=bits(base)),op('attribute-value'),op('reset-base'),op('attribute-value'),
                    op('notify-add'),op('attribute-value'),op('notify-remove'),op('attribute-value')])
    mutations=[[op('clear')],[op('put',instance=-1)],[op('queue-clear')],[op('queue-clear',target=1)],
               [op('put',key=-1,instance=0)],[op('queue-add',instance=-1)],[op('syncable',flag=False)]]
    for row in selected[:4]:
        for actions in mutations:
            for fail in [-1,0,1]:
                for at in [-1,0,1]:
                    yield dict(row,ops=[op('get'),op('base',value=bits(11)),op('reset-base',fail=fail,mutate=at,mutations=actions),
                                        op('value'),op('lookup'),op('attribute-value'),op('notify-remove'),op('reset-base')])
    rng=random.Random(0x4154545251554552)
    for _ in range(200):
        row=rng.choice(selected);n=len(row['hashes']);ops=[op('get')]
        for __ in range(40):
            kind=rng.choice(['attribute-has','modifier-has','attribute-value','attribute-base','modifier-value','reset-base',
                             'get','put','drop','base','prototype-base','prototype-permanent','prototype-remove','notify-add','notify-update','notify-remove'])
            ops.append(op(kind,key=rng.randrange(-1,n),template=rng.randrange(n),instance=rng.choice([0,-1]),value=bits(rng.choice(values)),
                          amplifier=rng.choice([-2**31,-1,0,1,255,2**31-1]),fail=rng.choice([-1,-1,-1,0,1]),
                          mutate=rng.choice([-1,-1,0,1,2]),mutations=rng.choice(mutations)))
        yield dict(row,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-attribute-map-queries-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaAttributeMapQueriesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/attribute_map_queries_fixture.rs']
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStoredEffectAttributesOracle.java'
    sources.extend([helper,lab.ROOT/'crates/horizon-world/examples/probe/stored_effect_attributes_fixture.rs',lab.ROOT/'tools/test_stored_effect_attributes_parity.py',lab.ROOT/'tools/test_stored_effect_notifications_parity.py'])
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaAttributeMapQueriesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';fixture_paths.append(enriched);tested_fixtures.update(base.source_hashes([enriched]))
        run([binary,'attribute_map_queries_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';candidate=build/f'rust-{side}.txt';matched=base.compare(reference,candidate)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched,'rust_output_sha256':lab.digest(candidate,'sha256')})
        print(f'{side}: {matched:,} attribute map query output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'fastutil_sha256':{str(p.relative_to(lab.ROOT)):lab.digest(p,'sha256') for p in libraries if 'fastutil' in str(p)},'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Actual AttributeMap hasAttribute/hasModifier/getValue/getBaseValue/getModifierValue/resetBaseValue using real cached AttributeInstances and mutable AttributeSupplier prototypes. Includes entire stored-notification corpus and checks no lazy creation on query/reset, supplier versus instance cache updates, modifier identity and missing-value exception categories, lookup aliasing/null mappings, captured reset instance, supplier base/modifier changes and dirty callback failure. Basic attribute default/syncability and entity onAttributeUpdated bodies remain controlled; registered ranged definitions, combined health/dimension consequences, full gameplay and platform builds remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped attribute map query parity passed: {report_path}')


if __name__=='__main__':main()
