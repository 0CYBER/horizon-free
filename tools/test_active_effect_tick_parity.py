#!/usr/bin/env python3
"""Compare inherited entity effect ticks, expirations, callbacks, blend and particles."""
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
    rng=random.Random(0x5449434b4546)
    durations=[-2147483648,-2,-1,0,1,2,599,600,601,1201,2147483647]
    floats=[0,0x80000000,0x3f000000,0x3f800000,0xbf800000,0x7f800000,0xff800000,0x7fc00000]
    doubles=[0,0x8000000000000000,0x3ff0000000000000,0xbff0000000000000,0x7ff0000000000000,0x7ff8000000000000]
    def state(**kw):
        v=dict(key=0,duration=20,amplifier=1,ambient=False,visible=True,icon=True,hidden=-1,factor=0x3f000000,previous=0);v.update(kw);return v
    def config(**kw):
        v=dict(states=[state(key=0,duration=1,hidden=-1),state(key=1,duration=601),state(key=2,duration=-1,hidden=0)],allow=True,level=0,client=False,removed=False,random=0,reseed=False,seeds=[9,-13],position=[1,2,3],category=0,tick=81,should=True,apply=True,advance=2,blend_in=4,blend_out=7,particles=[0,1,2],ambience=False,invisible=False,coordinates=[0,0x3ff0000000000000,0xbff0000000000000]);v.update(kw);return v
    def op(name,a=0,b=0,**kw):
        v=dict(op=name,a=a,b=b,source=-1,partial=0x3f000000,list=[0,1,2,0],probability=0x3f800000,sound=0,level=0);v.update(kw);return v
    def row(**kw):
        v=dict(xor=False,seeds=[1,-3],servers=[True,False],hashes=[0,1,2],initial=config(),after=config(states=[state(key=0,duration=1,amplifier=9),state(key=1,duration=-1,amplifier=2),state(key=2,duration=0,amplifier=0)],allow=True,level=1,client=True,removed=True,random=1,reseed=True,particles=[2,2,0],ambience=True,invisible=True,coordinates=[0x8000000000000000,0x7ff0000000000000,0x7ff8000000000000]),setup=[op('put',i,i) for i in range(3)],ops=[op('tick'),op('tick')],fail=-1,cme=-1,mutate=-1,mutations=[op('put',0,2)]);v.update(kw);return v
    for server in [False,True]:
        for should in [False,True]:
            for apply in [False,True]:
                for duration in durations:
                    for hidden in [-1,0]:
                        yield row(servers=[server,not server],initial=config(should=should,apply=apply,states=[state(key=0,duration=1201),state(key=1,duration=duration,hidden=hidden),state(key=2,duration=600)]),ops=[op('tick') for _ in range(4)])
        for mutation in [[],[op('drop',0)],[op('clear')],[op('put',2,0)],[op('drop',1),op('put',1,2)],[op('put',0,2)]]:
            for mutate in range(-1,10):
                for failure in ['fail','cme']:
                    for fail in range(-1,10):
                        yield row(servers=[server,not server],mutations=mutation,mutate=mutate,**{failure:fail})
    for xor in [False,True]:
        for ambient in [False,True]:
            for invisible in [False,True]:
                for seed in range(40):
                    yield row(xor=xor,seeds=[seed,-seed],servers=[False,False],setup=[],initial=config(ambience=ambient,invisible=invisible),ops=[op('tick') for _ in range(4)])
        for count in [0,1,2,8,9,13,25,49,64]:
            for server in [False,True]:
                states=[state(key=i,duration=[0,1,2,601,-1][i%5]) for i in range(count)]
                yield row(xor=xor,servers=[server,server],hashes=[0 for _ in range(count)],initial=config(states=states),after=config(states=states),setup=[op('put',i,i) for i in reversed(range(count))],ops=[op('tick') for _ in range(4)],mutations=[])
    for _ in range(600):
        count=rng.choice([3,5,8,16]);hashes=[rng.choice([0,0,64,i*31]) for i in range(count)]
        def states():return [state(key=rng.randrange(count),duration=rng.choice(durations),amplifier=rng.choice([-1,0,1,2,255,256,2147483647]),ambient=bool(rng.randrange(2)),visible=bool(rng.randrange(2)),icon=bool(rng.randrange(2)),hidden=rng.randrange(-1,i),factor=rng.choice(floats),previous=rng.choice(floats)) for i in range(count)]
        def cfg():return config(states=states(),allow=bool(rng.randrange(2)),level=rng.randrange(2),client=bool(rng.randrange(2)),removed=bool(rng.randrange(2)),random=rng.randrange(2),reseed=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],position=[rng.randrange(-2**31,2**31) for _ in range(3)],category=rng.randrange(8),tick=rng.randrange(-2**31,2**31),should=bool(rng.randrange(2)),apply=bool(rng.randrange(2)),advance=rng.choice(durations),blend_in=rng.choice(durations),blend_out=rng.choice(durations),particles=[rng.randrange(3) for _ in range(rng.randrange(5))],ambience=bool(rng.randrange(2)),invisible=bool(rng.randrange(2)),coordinates=[rng.choice(doubles) for _ in range(3)])
        yield row(xor=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],servers=[bool(rng.randrange(2)) for _ in range(2)],hashes=hashes,initial=cfg(),after=cfg(),setup=[op('put',rng.randrange(count),rng.randrange(count)) for _ in range(count)],ops=[op(rng.choice(['tick']*8+['apply-status','remove-status','clear-status','sound','add','force','remove','all','get','blend','put','drop','copy','alias']),rng.randrange(count),rng.randrange(count),source=rng.randrange(-1,2),partial=rng.choice(floats),list=[rng.randrange(count) for _ in range(rng.randrange(5))],probability=rng.choice(floats),level=rng.randrange(2),sound=rng.randrange(2)) for _ in range(20)],fail=rng.randrange(-1,30),cme=rng.randrange(-1,30),mutate=rng.randrange(-1,30),mutations=[op(rng.choice(['put','drop','clear','copy','alias']),rng.randrange(count),rng.randrange(count)) for _ in range(3)])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-active-effect-tick-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaActiveEffectTickOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/active_effect_tick_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaActiveEffectTickOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';enriched_hash=base.source_hashes([enriched])
        run([binary,'active_effect_tick_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';rust_output=build/f'rust-{side}.txt';matched=base.compare(reference,rust_output)
        base.ensure_sources_unchanged([enriched],enriched_hash)
        tested_fixtures.update(enriched_hash);fixture_paths.append(enriched)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'rust_output_sha256':lab.digest(rust_output,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} active effect tick output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Inherited LivingEntity.tickEffects composed with actual MobEffectInstance ticks, active-effect HashMaps, native synced particle/ambience data and both RNG kinds. Includes removal/refresh ordering, shared hidden restoration, nonstructural replacements, structural mutations, concurrent-modification catch boundaries, client blending, particle selection and RNG tails. Selected raw instance fields and callbacks can mutate during execution. Level classes are controlled ServerLevel and non-ServerLevel implementations; random particle positions, registered effect behavior and notification side effects remain controlled. Holder identity hashes are exported per reference run. Complete entity/player lifecycle and gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped active effect tick parity passed: {report_path}')


if __name__=='__main__':main()
