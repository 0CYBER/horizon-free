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
    # Real repeated server damage, with explicit clock advancement between hits.
    # Clock operations set up elapsed-time state; the enclosing native ticks are
    # not executed by this scene harness.
    for source in ['fall','stalagmite']:
        for absorption in [0,1,4,20]:
            for maximum in [1,20,40]:
                for gap in [0,1,9,10,11,19,20,21]:
                    ops=[op('attribute',bits(maximum),name='minecraft:max_health'),op('health',single(maximum)),
                         op('attribute',bits(20),name='minecraft:max_absorption'),op('absorption',single(absorption))]
                    for amount in [2,2,1,4,8,25]:
                        ops.append(op('hurt',single(amount),name='minecraft:'+source))
                        ops += [op('clock') for _ in range(gap)]
                    ops += [op('player_tick',vec((0,0,.98)),jumping=True),op('health',single(20)),
                            op('hurt',single(1),name='minecraft:'+source)]
                    yield dict(position=vec((25,4,25)),ops=ops)
    for source in ['fall','stalagmite']:
        for armor in [0,5,20,30]:
            for toughness in [0,8,20]:
                for amount in [-1,-0.,0.,1e-30,.1,5,20,1e30,float('inf'),float('nan')]:
                    ops=[op('attribute',bits(armor),name='minecraft:armor'),op('attribute',bits(toughness),name='minecraft:armor_toughness'),
                         op('attribute',bits(100),name='minecraft:max_health'),op('health',single(100)),
                         op('hurt',single(amount),name='minecraft:'+source),op('hurt',single(5),name='minecraft:'+source)]
                    yield dict(position=vec((25,4,25)),ops=ops)
    for name,properties in [('pointed_dripstone',{'vertical_direction':'up','thickness':'tip'}),('turtle_egg',{'eggs':'4','hatch':'2'})]:
        for seed in range(8):
            yield dict(position=vec((25.5,4,25.5)),ops=[op('seed',seed),op('block',name='minecraft:'+name,position=[25,3,25],properties=properties),
                *[v for _ in range(8) for v in [op('fall',bits(8)),op('landing',bits(-.1),ground=True,position=[25,3,25])]],
                op('block',name='minecraft:stone',position=[25,3,25])])
    for safe in [0,1,3,6,20]:
        for multiplier in [0,.5,1,2]:
            for impact in [None,3,4,4.1,7,18]:
                ops=[op('attribute',bits(safe),name='minecraft:safe_fall_distance'),op('attribute',bits(multiplier),name='minecraft:fall_damage_multiplier'),
                     op('impulse',None if impact is None else vec((25.5,impact,25.5)),ignore=True),op('fall',bits(12)),
                     op('landing',bits(-.125),ground=True,position=[25,3,25])]
                yield dict(position=vec((25.5,4,25.5)),ops=ops)
    for name in ['stone','hay_block','honey_block','slime_block','powder_snow','pointed_dripstone','farmland','turtle_egg','shelf_mushroom']:
        for seed in range(4):
            for distance in [0,.25,2,2.999999,3,3.000001,4,7,20]:
                ops=[op('seed',seed),op('block',name='minecraft:'+name,position=[25,3,25]),op('fall',bits(distance)),
                     op('landing',bits(-.125),ground=True,position=[25,3,25]),op('landing',bits(0),ground=True,position=[25,3,25]),
                     op('block',name='minecraft:stone',position=[25,3,25])]
                yield dict(position=vec((25.5,3.9375 if name=='farmland' else 4,25.5)),ops=ops)
    for name in ['stone','hay_block','honey_block','slime_block','farmland','turtle_egg']:
        for height in [4,5,7,8,11,18,40]:
            ops=[op('block',name='minecraft:'+name,position=[25,3,25])]
            ops += [op('player_tick',vec((0,0,0)),jumping=False) for _ in range(100)]
            ops.append(op('block',name='minecraft:stone',position=[25,3,25]))
            yield dict(position=vec((25.5,height,25.5)),ops=ops)
    for start in [(25,4,25),(25,18,25),(-11,4,0),(-7,5,14),(0,18,0)]:
        for yaw in [0,45,90,-135,180]:
            for trigger in [-1,0,1,7]:
                ops=[op('yaw',single(yaw)),op('trigger',trigger)]
                for i in range(180):
                    if i in [20,80]:ops.append(op('sprint',i==20))
                    if i==120:ops.append(op('attribute',bits(.2),name='minecraft:movement_speed'))
                    ops.append(op('player_tick',vec((.7071067690849304 if i<45 else 0,0,.7071067690849304 if i<45 else .98 if i<90 else 0)),jumping=(10<=i<60 or 120<=i<150)))
                yield dict(position=vec(start),ops=ops)
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
        'VanillaFaceSupportOracle','VanillaEntityAttributeOracle','VanillaBlockLandingOracle','VanillaMovementRegistryOracle','VanillaDamageSourceOracle']]
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
    fixtures=list(corpus());operations=sum(len(f['ops']) for f in fixtures);expected_lines=4*operations+len(fixtures)
    (build/'inputs.txt').write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures))
    input_hash=lab.digest(build/'inputs.txt','sha256')
    damage_dir=build/'damage';damage_dir.mkdir(exist_ok=True)
    for target, source in [('types.json','damage-types.json'),('tags.json','damage-tags.json')]:
        (damage_dir/target).write_bytes((resources/source).read_bytes())
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
        'operations':operations,'sources_sha256':tested,'resources':resource_hash,'input_sha256':input_hash,
        'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,
        'scope':'Stored PlayerMotion binds resident client chunks and registered attributes/coefficients to air travel, jumping, Entity.move, step resolution, ground/support updates, restitution and speed factors. Repeated native Player.travel/jump/move and actual Player.aiStep / direct LivingEntity.aiStep compare position, velocity, bounds, collision/ground flags, support, climb state, fall-reset field, both sync flags, speed, head yaw, jump trigger and jump delay by exact bits. Prepared-local-input Player.aiStep paths verify the stored speed refresh after movement, with empty inventory and no nearby entities. Travel uses the actual Player.getSpeed movement-attribute query, independently of that stored base-living speed field; both getter and setter execute native bodies. Dry aiStep includes small-velocity cleanup, input damping, held/released jump timing and server frost-modifier removal. Snapshots also compare all allocated attribute instances, base/cache/dirty state, modifier values/operations/permanence and pending update/sync membership (not JVM identity-dependent iteration order). Dry resident scene, fixed standing dimensions, zero frozen ticks, empty equipment, no interpolation, fluids, vehicles, effects, crouching or other entities. Actual LivingEntity.checkFallDamage, registered Block.fallOn/bounceOn and Player/LivingEntity.causeFallDamage are composed, including float-narrowed fall-distance accumulation, reset, last-climb clearing, safe-fall and damage-multiplier attributes, impulse exemptions and ordered landing particle/sound/statistic/hit-ground/damage requests. Farmland pushes the stored player and replaces actual chunk state; repeated turtle-egg changes preserve hatch. Block state at the landing fixture is compared after every operation. World RNG is a seeded native legacy stream. Actual ServerPlayer/Player/LivingEntity hurt dispatch now updates native synchronized health and player absorption, modified armor/toughness reduction, exhaustion, damage cooldown/last-source state and velocity synchronization. Native health storage and dirty flags are compared after every operation. Damage sources use actual decoded/tagged pinned damage registries. ServerPlayer jump statistics and exhaustion are included; dead-or-dying state disables movement input and jumping. Empty equipment/hands/effects, normal difficulty, damage gamerules enabled, loaded client and no spawn-extra fall particles define this scene domain. Ordered health/statistic/combat-record/broadcast/sound/criterion/death requests are compared; criterion argument payloads retain the separate combat gate. Actual health changes are applied, but audiovisual playback, persistent statistics, combat tracker internals, full death/drop/respawn services, block neighbor/drop/block-entity callbacks, movement emission, block-inside and networking remain controlled consumers. Explicit clock operations establish cooldown/hurt-time/game-time state using the two damage-clock phases; the full enclosing native ticks are not invoked by those setup operations. Input preparation has its own client comparison. No full Player.tick, hunger regeneration, local input lifecycle, nonempty inventory/equipment ticks, full freezing, complete death lifecycle or full gameplay parity claim.'})
    print(f'Scoped scene motion parity passed: {report}',flush=True)

if __name__=='__main__':main()
