#!/usr/bin/env python3
"""Compare concrete consume effects composed with inherited entity status storage."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    rng=random.Random(0x53544f524544)
    durations=[-2147483648,-2,-1,0,1,20,200,2147483647]
    amps=[-1,0,1,2,255,256,2147483647]
    floats=[0,0x80000000,0x3f000000,0x3f800000,0xbf800000,0x7f800000,0xff800000,0x7fc00000]
    def state(**kw):
        v=dict(key=0,duration=20,amplifier=1,ambient=False,visible=True,icon=True,hidden=-1,factor=0x3f000000,previous=0);v.update(kw);return v
    def config(**kw):
        v=dict(states=[state(),state(duration=200,amplifier=0),state(key=1,hidden=0)],allow=True,level=0,client=False,removed=False,random=0,reseed=False,seeds=[9,-13],position=[1,2,3],category=0);v.update(kw);return v
    def op(name,a=0,b=0,**kw):
        v=dict(op=name,a=a,b=b,source=-1,partial=0x3f000000,list=[0,1,2,0],probability=0x3f800000,sound=0,level=0);v.update(kw);return v
    def row(**kw):
        v=dict(xor=False,seeds=[1,-3],hashes=[0,1,2],initial=config(),after=config(states=[state(duration=1,amplifier=9),state(duration=-1,amplifier=2),state(duration=0,amplifier=0)],allow=True,level=1,client=True,removed=True,random=1,reseed=True,position=[-4,5,-6],category=4),setup=[],ops=[op('apply-status'),op('remove-status'),op('clear-status')],fail=-1,mutate=-1,mutations=[op('put',0,2)]);v.update(kw);return v
    for xor in [False,True]:
        for name in ['apply-status','remove-status','clear-status','sound']:
            for setup in [[],[op('put',0,0)],[op('put',0,1),op('put',1,2)]]:
                for fail in range(-1,9):
                    for mutate in range(-1,9):
                        yield row(xor=xor,setup=setup,ops=[op(name),op('get'),op('has')],fail=fail,mutate=mutate)
        for probability in floats+[0x3eaaaaab,0x3f7fffff,1,0xffffffff]:
            for ids in [[],[0],[0,0],[0,1,2],[2,0,1,2,0]]:
                for allow in [False,True]:
                    yield row(xor=xor,initial=config(allow=allow),ops=[op('apply-status',list=ids,probability=probability),op('apply-status',list=ids,probability=probability),op('remove-status',list=list(reversed(ids))),op('clear-status')])
        for count in [1,2,3,7,8,9,11,13,24,25,48,49,64]:
            for collision in [False,True]:
                states=[state(key=i,duration=200,amplifier=i%3) for i in range(count)]
                yield row(xor=xor,hashes=[0 if collision else i*31 for i in range(count)],initial=config(states=states),after=config(states=states),setup=[],ops=[op('apply-status',list=list(reversed(range(count)))),op('remove-status',list=list(range(0,count,3))),op('clear-status')],mutations=[])
    for _ in range(600):
        count=rng.choice([3,5,8,16]);hashes=[rng.choice([0,0,64,i*31]) for i in range(count)]
        def states():return [state(key=rng.randrange(count),duration=rng.choice(durations),amplifier=rng.choice(amps),ambient=bool(rng.randrange(2)),visible=bool(rng.randrange(2)),icon=bool(rng.randrange(2)),hidden=rng.randrange(-1,i),factor=rng.choice(floats),previous=rng.choice(floats)) for i in range(count)]
        def cfg():return config(states=states(),allow=bool(rng.randrange(2)),level=rng.randrange(2),client=bool(rng.randrange(2)),removed=bool(rng.randrange(2)),random=rng.randrange(2),reseed=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],position=[rng.randrange(-2**31,2**31) for _ in range(3)],category=rng.randrange(8))
        yield row(xor=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],hashes=hashes,initial=cfg(),after=cfg(),setup=[op('put',rng.randrange(count),rng.randrange(count)) for _ in range(count)],ops=[op(rng.choice(['apply-status','apply-status','remove-status','clear-status','sound','add','force','remove','remove-raw','all','has','get','blend','put','drop','copy','alias']),rng.randrange(count),rng.randrange(count),source=rng.randrange(-1,2),partial=rng.choice(floats),list=[rng.randrange(count) for _ in range(rng.randrange(7))],probability=rng.choice(floats),level=rng.randrange(2),sound=rng.randrange(2)) for _ in range(20)],fail=rng.randrange(-1,30),mutate=rng.randrange(-1,30),mutations=[op(rng.choice(['put','drop','clear','copy','alias']),rng.randrange(count),rng.randrange(count)) for _ in range(3)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-stored-consume-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStoredConsumeOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/stored_consume_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaStoredConsumeOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';enriched_hash=base.source_hashes([enriched])
        run([binary,'stored_consume_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';rust_output=build/f'rust-{side}.txt';matched=base.compare(reference,rust_output)
        base.ensure_sources_unchanged([enriched],enriched_hash)
        tested_fixtures.update(enriched_hash);fixture_paths.append(enriched)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'rust_output_sha256':lab.digest(rust_output,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} stored consume output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Concrete apply/remove/clear/sound consume effects composed with inherited LivingEntity effect storage, actual status instances and JDK maps. Both RNG kinds, probability edges, ordinary instance copies, hidden updates, duplicate effects, source identity, map iteration/copy order, callback mutation and failure prefixes are compared. Native holder hashes and identity hashes are captured per reference run. Eligibility, registered effect bodies, attribute/packet notifications and sound delivery remain controlled. Full consumable/player lifecycle and gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped stored consume parity passed: {report_path}')


if __name__=='__main__':main()
