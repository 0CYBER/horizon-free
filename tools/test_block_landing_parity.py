#!/usr/bin/env python3
"""Compare every registered block landing/bounce family with pinned vanilla."""
import json
import math
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(states, definitions):
    def bits(v):return struct.pack('>d',float(v)).hex()
    def single(v):return struct.pack('>f',float(v)).hex()
    def row(state,kind=2,**kw):
        f=dict(state=state,kind=kind,bounce=False,server=True,client=False,distance=bits(8),random_float=single(.25),random_int=0,width=single(.6),height=single(1.8),damage=True,suppress=False,protected=False,griefing=True,write=True,position=[17,-64,-33],entities=[],mutate=False,fail=-1)
        f.update(kw);return f
    for state in states:
        for kind in range(8):
            for bounce in [False,True]:yield row(state['id'],kind,bounce=bounce)
    special=[s for s in states if definitions[s['name']]['fall']!='Block' or definitions[s['name']]['bounce']]
    distances=[-math.inf,-1.,-0.,0.,.5,math.nextafter(.75,0),.75,math.nextafter(.75,1),1.,math.nextafter(4.,0),4.,math.nextafter(4.,5),math.nextafter(7.,0),7.,math.nextafter(7.,8),1e40,math.inf,math.nan]
    for state in special:
        for kind in range(8):
            for distance in distances:
                for server,client in [(True,False),(True,True),(False,True),(False,False)]:yield row(state['id'],kind,distance=bits(distance),server=server,client=client)
            for flag in ['damage','suppress','protected','griefing','write']:
                for value in [False,True]:
                    for fail in range(-1,18):yield row(state['id'],kind,**{flag:value,'fail':fail})
            for bounce in [False,True]:
                for fail in range(-1,4):yield row(state['id'],kind,bounce=bounce,fail=fail)
    rng=random.Random(0x4c414e44494e47)
    for _ in range(18000):
        state=rng.choice(special);p=rng.choice([[0,0,0],[17,-64,-33],[2147483647,0,-2147483648]])
        entities=[]
        if definitions[state['name']]['fall']=='FarmlandBlock':
            for _ in range(rng.randrange(5)):
                y=rng.choice([0,.5,.9375,1,1.1]);low=[p[0]+.2,p[1]+y,p[2]+.2];entities.append([*low,low[0]+.6,low[1]+1.8,low[2]+.6])
        yield row(state['id'],rng.randrange(8),server=bool(rng.randrange(2)),client=bool(rng.randrange(2)),distance=bits(rng.choice(distances+[rng.uniform(-1,12)])),random_float=single(rng.choice([0.,.25,.5,.99999994,math.nan])),random_int=rng.randrange(3),width=single(rng.choice([0.,.6,.8,1.,2.,math.nan,math.inf])),height=single(rng.choice([0.,.8,1.,1.8,math.nan,math.inf])),damage=bool(rng.randrange(2)),suppress=bool(rng.randrange(2)),protected=bool(rng.randrange(2)),griefing=bool(rng.randrange(2)),write=bool(rng.randrange(2)),position=p,entities=[[bits(v) for v in b] for b in entities],mutate=bool(rng.randrange(2)),fail=rng.choice([-1,-1,-1,0,1,2,3,4,5,6,7,8,9,10,11,12]))
    # Complete successful farmland pushes, all mutation/failure cut points and volume threshold.
    farms=[s['id'] for s in states if definitions[s['name']]['fall']=='FarmlandBlock']
    for state in farms:
        for width,height in [(.8,.8),(.8,math.nextafter(.8,1)),(.6,1.8),(1.,.512),(1.,.5120001),(1.,.5119999),(-.8,1.),(math.inf,0.),(math.nan,1.)]:
            for kind in [1,2]:
                for fail in range(-1,18):
                    for mutate in [False,True]:yield row(state,kind,width=single(width),height=single(height),entities=[[bits(v) for v in b] for b in [[17.2,-63.0625,-32.8,17.8,-61.2,-32.2],[17.2,-63.5,-32.8,17.8,-61.7,-32.2]]],mutate=mutate,fail=fail)


def main():
    lock,jars,libraries=base.verify_artifacts();build=lab.LOCAL/'rust-block-landing-parity';build.mkdir(exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    root=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity'
    oracles=[root/(n+'.java') for n in ['VanillaBlockLandingOracle','VanillaMovementRegistryOracle','VanillaHeightmapOracle','VanillaCollisionOracle']]
    sources=[*oracles,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/examples/probe/block_landing_fixture.rs']
    for crate in ['horizon-core','horizon-world']:sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested=base.source_hashes(sources);classes=build/'classes';classes.mkdir(exist_ok=True);dependencies=os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,mode):run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.VanillaBlockLandingOracle',mode,build],side+'-'+mode)
    java('server','export');states=json.loads((build/'landing-states.json').read_text());definitions=json.loads((build/'landing-definitions.json').read_text())
    if len(states)!=35723 or len(definitions)!=1286:raise AssertionError('Incomplete pinned registry')
    count=0
    with (build/'inputs.txt').open('w') as output:
        for f in corpus(states,definitions):output.write(json.dumps(f,separators=(',',':'))+'\n');count+=1
    fixture_paths=[build/n for n in ['inputs.txt','landing-states.json','landing-definitions.json','movement-properties.nbt','block-registry.nbt','cached-collision.nbt']];fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'block_landing_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side);matched=base.compare(build/(side+'.txt'),build/'rust.txt');references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),output_sha256=lab.digest(build/(side+'.txt'),'sha256'),matched_output_lines=matched));print(f'{side}: {matched:,} registered block landing lines match',flush=True)
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report=dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),registry_states=len(states),registry_blocks=len(definitions),fixtures=count,sources_sha256=tested,fixtures_sha256=fixtures,rust_output_sha256=lab.digest(build/'rust.txt','sha256'),references=references,scope='Actual registered Block.fallOn and bounceOn across all states and eight entity class families, plus client/server distinctions, damage return, bounce suppression, protection/rules, random boundaries, NaN/infinity and failures at each observable callback. Farmland replacement uses native registered collision shapes and ordered occupant pushes; turtle egg changes preserve hatch and compare write flags, event order and effects after failed writes. Entity damage, randomness, selected entities, sounds, world writes and events are controlled providers. Live health, entity storage, network, audio and full movement lifecycle integration remain separate work.')
    lab.write_json(report_path,report);print(f'Scoped registered landing parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
