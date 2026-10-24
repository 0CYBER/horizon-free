#!/usr/bin/env python3
"""Compare entity effect notifications with real stored attributes and native pending refresh."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
try:
    from . import lab, test_rust_parity as base, test_stored_effect_attributes_parity as stored_base
except ImportError:
    import lab
    import test_rust_parity as base
    import test_stored_effect_attributes_parity as stored_base


def corpus():
    rows=list(stored_base.corpus())
    yield from rows
    def op(kind, **kw):
        r=dict(op=kind,key=0,instance=0,template=0,target=0,amplifier=0,flag=False,value='0000000000000000',
               fail=-1,mutate=-1,mutations=[],client=False,reapply=True,count=1)
        r.update(kw);return r
    selected=[rows[i] for i in [0,4,8,12,18,22,36,50]]
    for row in selected:
        for client in [False,True]:
            for reapply in [False,True]:
                for count in [0,1,3]:
                    yield dict(row,ops=[op('get'),op('dirty'),op('notify-add',client=client),op('value'),
                        op('notify-update',amplifier=4,reapply=reapply,client=client),op('value'),
                        op('notify-remove',count=count,client=client),op('refresh'),op('value')])
    actions=[[op('queue-clear')],[op('queue-add',instance=-1)],[op('queue-drop',instance=0)],
             [op('queue-add',target=1,instance=-1)],[op('clear')],[op('put',instance=0)],
             [op('drop')],[op('syncable',flag=False)],[op('syncable',flag=True)]]
    for row in selected[:5]:
        for kind in ['notify-add','notify-update','notify-remove','refresh']:
            for fail in range(12):
                yield dict(row,ops=[op('get'),op('notify-add'),op('dirty'),op(kind,fail=fail),op('value'),op('refresh'),op('notify-remove')])
            for mutations in actions:
                for at in [0,1,2,3,5]:
                    yield dict(row,ops=[op('get'),op('notify-add'),op('dirty'),op(kind,mutate=at,mutations=mutations),op('value'),op('refresh'),op('notify-remove')])
    rng=random.Random(0x53544f524e4f54)
    for _ in range(150):
        row=rng.choice(selected)
        ops=[op('get')]
        for __ in range(20):
            kind=rng.choice(['notify-add','notify-update','notify-remove','refresh','dirty','value','queue-clear'])
            ops.append(op(kind,client=rng.randrange(7)==0,reapply=bool(rng.randrange(2)),count=rng.randrange(4),
                          amplifier=rng.choice([-2**31,-1,0,1,255,256,2**31-1]),flag=bool(rng.randrange(2)),
                          fail=rng.choice([-1,-1,-1,0,1,3,5]),mutate=rng.choice([-1,-1,0,1,2,3,5]),mutations=rng.choice(actions)))
        yield dict(row,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-stored-effect-notifications-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStoredEffectNotificationsOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/stored_effect_notifications_fixture.rs']
    helper=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStoredEffectAttributesOracle.java'
    sources.extend([helper,lab.ROOT/'crates/horizon-world/examples/probe/stored_effect_attributes_fixture.rs',lab.ROOT/'tools/test_stored_effect_attributes_parity.py'])
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,helper],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaStoredEffectNotificationsOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';fixture_paths.append(enriched);tested_fixtures.update(base.source_hashes([enriched]))
        run([binary,'stored_effect_notifications_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';candidate=build/f'rust-{side}.txt';matched=base.compare(reference,candidate)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched,'rust_output_sha256':lab.digest(candidate,'sha256')})
        print(f'{side}: {matched:,} stored effect notification output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'fastutil_sha256':{str(p.relative_to(lab.ROOT)):lab.digest(p,'sha256') for p in libraries if 'fastutil' in str(p)},'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Inherited LivingEntity onEffectAdded/onEffectUpdated/onEffectsRemoved and private refreshDirtyAttributes, with actual MobEffect modifier bodies, AttributeMap.getInstance, AttributeSupplier.createInstance, AttributeInstance storage and native pending update/sync sets. Includes complete stored-attribute corpus plus client guards, effect dirty state, reapply/empty removal, refresh failure prefixes and callback map/queue mutation. Actual JVM identity hashes replayed separately in Rust. Virtual onAttributeUpdated consequence, basic attribute default/syncability methods and level client flag are controlled; passenger lists are empty (packet paths have separate gates). Registered definitions, combined health/dimension consequences, full entity lifecycle/gameplay and platform builds remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped stored effect notification parity passed: {report_path}')


if __name__=='__main__':main()
