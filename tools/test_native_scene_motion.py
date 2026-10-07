#!/usr/bin/env python3
"""Compare composed stored player movement over the native client's resident scene."""
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base, test_native_client as client
except ImportError:
    import lab
    import test_rust_parity as base
    import test_native_client as client

def bits(value): return struct.pack('>d', value).hex()
def single(value): return struct.pack('>f', value).hex()
def vec(values): return list(map(bits, values))
def op(mode, value=None, **extra): return dict(mode=mode, value=value, **extra)

def corpus():
    for permanent in [False,True]:
        for operation in range(3):
            for value in [-.05,0,.2]:
                yield dict(position=vec((25,4,25)),ops=[
                    op('modifier',bits(value),name='minecraft:movement_speed',id='minecraft:powder_snow',operation=operation,permanent=permanent),
                    op('sprint',True),op('ack_attributes'),op('tick',vec((0,0,1))),
                    op('ack_attributes'),op('tick',vec((0,0,1))),op('sprint',False),op('tick',vec((0,0,1)))])
    # Native LivingEntity.aiStep drives repeated held/released jumps and the
    # player-specific horizontal velocity threshold, including delay boundaries.
    for start in [(25,4,25),(-11,4,0),(-7,4,26),(0,18,0)]:
        for delay in [-1,0,1,9,10,11]:
            ops=[op('speed',single(.1)),op('delay',delay),op('jumping',True)]
            for i in range(180):
                if i in [35,120]:ops.append(op('jumping',False))
                if i in [36,140]:ops.append(op('jumping',True))
                ops.append(op('tick',vec((.5 if i<90 else 0,0,0))))
            yield dict(position=vec(start),ops=ops)
    for x in [0,-0.0,.002,.0021213203435596424,.0029999999999999996,.003,.004]:
        for z in [0,.002,.003,-.002]:
            for y in [-.003,-.0029999999999999996,-0.0,0,.0029999999999999996,.003]:
                yield dict(position=vec((25,12,25)),ops=[op('velocity',vec((x,y,z))),op('tick',vec((0,0,0))),op('tick',vec((0,0,0)))])
    for start in [(25,18,25),(0,18,0),(-15.9,4,-15.9),(-11,4,0),(5,4,14),(-7,4,26)]:
        for yaw in [0,45,90,180,-135]:
            for sprint in [False,True]:
                ops=[op('speed',single(.1)),op('yaw',single(yaw)),op('sprint',sprint)]
                for i in range(120):
                    if i in [15,45,80]:ops.append(op('jump'))
                    ops.append(op('travel',vec((.98 if 35<=i<65 else 0,0,.98 if i<95 else 0))))
                yield dict(position=vec(start),ops=ops)
    for index in range(16):
        x,z=index%4*4-7,index//4*4+14
        for dx,dz in [(1,0),(-1,0),(0,1),(0,-1)]:
            for y in [4,4.5,6]:
                ops=[op('speed',single(.1)),op('velocity',vec((dx*.2,-.1,dz*.2))),op('fall',bits(4)),op('jumping',True)]
                for i in range(48):
                    if i==20:ops.append(op('jump'))
                    ops.append(op('travel',vec((dx*.5,0,dz*.5))))
                yield dict(position=vec((x+.5-dx,y,z+.5-dz)),ops=ops)
    for attribute,values in {
        'gravity':[0,.01,.08,.2,1], 'friction_modifier':[0,.5,1,2], 'air_drag_modifier':[0,.5,1,2],
        'bounciness':[0,.25,1], 'jump_strength':[0,.1,.42,1.5], 'step_height':[0,.5,.6,1,1.5],
        'movement_efficiency':[0,.5,1],
    }.items():
        for value in values:
            for start in [(25,8,25),(-11,4,0),(-7.5,5,14.5)]:
                ops=[op('attribute',bits(value),name='minecraft:'+attribute),op('speed',single(.1)),op('fall',bits(3))]
                for i in range(100):
                    if i in [10,40]:ops.append(op('jump'))
                    ops.append(op('travel',vec((.98,0,0))))
                yield dict(position=vec(start),ops=ops)
    rng=random.Random(0x4d4f54494f4e)
    for _ in range(300):
        start=(rng.uniform(-17,30),rng.uniform(3,15),rng.uniform(-17,30))
        ops=[op('speed',single(rng.choice([0,.1,.13,.2])))]
        for _ in range(50):
            choice=rng.randrange(12)
            if choice==0:ops.append(op('jump'))
            elif choice==1:ops.append(op('sprint',rng.choice([True,False])))
            elif choice==2:ops.append(op('yaw',single(rng.uniform(-720,720))))
            elif choice==3:ops.append(op('velocity',vec([rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-1,1)])))
            elif choice==4:ops.append(op('stuck',vec(rng.choice([(0,0,0),(.25,.05,.25),(.9,1.5,.9)]))))
            else:ops.append(op('travel',vec((rng.choice([-1,0,.98]),0,rng.choice([-1,0,.98])))))
        yield dict(position=vec(start),ops=ops)
    # Long histories exercise the native bounded movement-record retention.
    yield dict(position=vec((25,4,25)),ops=[op('speed',single(.1))]+[op('travel',vec((0,0,0))) for _ in range(400)])

def main():
    lock,jars,libraries=base.verify_artifacts()
    build=lab.LOCAL/'rust-native-scene-motion';build.mkdir(exist_ok=True)
    report=build/'report.json';report.unlink(missing_ok=True)
    resources=lab.LOCAL/'horizon-client'/lock['minecraft']['version']
    resource_hash=client.resource_fingerprint(resources)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSceneMotionOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaSceneCollisionOracle','VanillaRegistryCollisionScanOracle',
        'VanillaHeightmapOracle','VanillaCollisionOracle','VanillaBlockStateCodecOracle','VanillaSuffocationOracle',
        'VanillaFaceSupportOracle','VanillaEntityAttributeOracle']]
    sources=[Path(__file__),lab.ROOT/'tools/test_native_client.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',
             lab.ROOT/'tools/prepare_native_client.py',oracle,*helpers,lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',
             lab.ROOT/'crates/horizon-client/examples/scene_motion_probe.rs']
    for crate in ['horizon-core','horizon-world','horizon-assets','horizon-client']:
        sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested=base.source_hashes(sources)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:
            subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    dependencies=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-cp',str(jars['server'])+os.pathsep+dependencies,
         '-d',classes,oracle,*helpers],'compile-java')
    fixtures=list(corpus());expected_lines=sum(len(f['ops'])+1 for f in fixtures)
    (build/'inputs.txt').write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures))
    input_hash=lab.digest(build/'inputs.txt','sha256')
    run(['cargo','build','--locked','-p','horizon-client','--example','scene_motion_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT))
    binary=Path(metadata['target_directory'])/'debug/examples'/('scene_motion_probe.exe' if os.name=='nt' else 'scene_motion_probe')
    run([binary,resources,build],'rust')
    references=[]
    for side,jar in jars.items():
        run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),
             'dev.horizonlab.parity.VanillaSceneMotionOracle',side,build,resources],side)
        matched=base.compare(build/(side+'.txt'),build/'rust.txt')
        if matched!=expected_lines:raise AssertionError('Incomplete motion output')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} evolving scene motion states match',flush=True)
    base.ensure_sources_unchanged(sources,tested)
    if resource_hash!=client.resource_fingerprint(resources) or input_hash!=lab.digest(build/'inputs.txt','sha256'):
        raise AssertionError('Motion inputs changed during verification')
    lab.write_json(report,{'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],
        'platform':platform.platform(),'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
        'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'fixtures':len(fixtures),
        'operations':expected_lines-len(fixtures),'sources_sha256':tested,'resources':resource_hash,'input_sha256':input_hash,
        'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,
        'scope':'Stored PlayerMotion binds resident client chunks and registered attributes/coefficients to air travel, jumping, Entity.move, step resolution, ground/support updates, restitution and speed factors. Repeated native Player.travel/jump/move and direct LivingEntity.aiStep compare position, velocity, bounds, collision/ground flags, support, climb state, fall-reset field, both sync flags, speed and jump delay by exact bits. Dry aiStep includes small-velocity cleanup, input damping, held/released jump timing and server frost-modifier removal. Snapshots also compare all allocated attribute instances, base/cache/dirty state, modifier values/operations/permanence and pending update/sync membership (not JVM identity-dependent iteration order). Dry resident scene, fixed standing dimensions, zero frozen ticks, empty equipment, no interpolation, fluids, vehicles, effects, crouching or other entities. Landing/damage, audiovisual emission, block-inside and network consumers remain controlled boundaries. No outer Player.aiStep/tick, local input controller, full freezing, health/damage or full gameplay parity claim.'})
    print(f'Scoped scene motion parity passed: {report}',flush=True)

if __name__=='__main__':main()
