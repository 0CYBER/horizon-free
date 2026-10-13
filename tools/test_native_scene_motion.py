#!/usr/bin/env python3
"""Compare composed stored player movement over the native client's resident scene."""
import argparse
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

def attribute_corpus(controlled):
    def tick(keys=0):
        return op('controlled_tick',keys) if controlled else op('simulation_tick',vec((0,0,.98 if keys&1 else 0)),jumping=bool(keys&16))
    def attr(name,value):return op('attribute',bits(value),name='minecraft:'+name)
    # Limits stay pending until explicit refresh or the simulation's post-move
    # phase. Repeated callbacks must clear updates but preserve sync membership.
    for name in ['max_health','max_absorption','scale','waypoint_receive_range','waypoint_transmit_range']:
        for value in [-float('inf'),-1,-0.,0.,.125,.5,1,2,4,20,float('inf'),float('nan')]:
            ops=[attr('max_absorption',20),op('absorption',single(15)),op('refresh_attributes'),
                 attr(name,value),op('refresh_attributes'),op('refresh_attributes'),attr(name,1),tick(),tick(),op('refresh_attributes')]
            yield dict(position=vec((25.5,4,25.5)),ops=ops)
    rng=random.Random(0x41545452)
    names=['max_health','max_absorption','scale','waypoint_receive_range','waypoint_transmit_range']
    for _ in range(100):
        ops=[]
        for cycle in range(4):
            ops += [attr('max_health',40),op('health',single(35)),attr('max_absorption',20),op('absorption',single(15))]
            rng.shuffle(names)
            for name in names:ops.append(attr(name,rng.choice([0,.5,1,2,4])))
            ops += [tick(rng.choice([0,1,33,49,65])),tick(),op('refresh_attributes')]
        yield dict(position=vec((25.5,4,25.5)),ops=ops)
    # Modifier changes feed the same callbacks; rebuilding dimensions must use
    # the computed value without replacing or clearing active modifiers.
    for name in ['scale','max_health','max_absorption']:
        for operation in range(3):
            for permanent in [False,True]:
                ops=[attr('max_absorption',20),op('absorption',single(15)),op('refresh_attributes'),
                     op('modifier',bits(-.5),name='minecraft:'+name,id='horizon:fixture',operation=operation,permanent=permanent),
                     tick(),op('refresh_attributes'),tick(1),tick(0)]
                yield dict(position=vec((25.5,4,25.5)),ops=ops)
    # Walk through a low tunnel, release Shift inside, then leave it; force a
    # one-block passage by replacing the roof around a crouched player.
    for roof,properties in [('stone_slab',{'type':'top'}),('stone',{})]:
        for scale in [.5,1,1.25]:
            for pose in ['STANDING','CROUCHING','SWIMMING']:
                blocks=[(x,5,z) for x in range(24,27) for z in range(24,30)]
                ops=[attr('scale',scale),op('refresh_attributes'),op('pose',pose)]
                ops += [op('block',name='minecraft:'+roof,position=list(p),properties=properties) for p in blocks]
                for i in range(140):
                    keys=33 if i<30 else 1 if i<110 else 0
                    if i==60:ops.append(attr('scale',1))
                    if not controlled:ops.append(op('shift',bool(keys&32)))
                    ops.append(tick(keys))
                ops += [op('block',name='minecraft:air',position=list(p)) for p in blocks]
                yield dict(position=vec((25.5,4,25.5)),ops=ops)

def pose_corpus(controlled):
    def ticks(keys, count):
        if controlled:
            return [op('controlled_tick', keys) for _ in range(count)]
        return [op('shift', bool(keys & 32))] + [op('simulation_tick',vec((.98 if keys & 4 else 0,0,.98 if keys & 1 else 0)),jumping=bool(keys & 16)) for _ in range(count)]
    # The actor starts on ordinary resident ground. Change the ceiling in place
    # to exercise all fit fallbacks, including a space too small to change pose.
    for scale in [.5,1,1.25,2]:
        for sneak in [0,.3,.5,1]:
            ops=[op('attribute',bits(scale),name='minecraft:scale'),op('refresh_dimensions'),
                 op('attribute',bits(sneak),name='minecraft:sneaking_speed')]
            for ceiling,props,y in [('air',{},5),('stone_slab',{'type':'top'},5),('stone',{},5),('stone',{},4)]:
                ops += [op('block',name='minecraft:'+ceiling,position=[25,y,25],properties=props),op('pose_tick')]
                ops += ticks(32,3)+ticks(0,3)
                ops += [op('block',name='minecraft:air',position=[25,y,25]),op('pose_tick')]
            for pose in ['CROUCHING','SWIMMING','STANDING']:
                ops += [op('pose',pose)]+ticks(0,2)
            yield dict(position=vec((25.5,4,25.5)),ops=ops)
    # Finite-scene edges are open air: axis/diagonal backoff, step-height
    # thresholds, falling grace, key release, sprint and jump combinations.
    for start in [(30.9,4,30.9),(30.9,4.3,30.9),(30.9,4.6,30.9),(-15.9,4,-15.9)]:
        for step in [0,.5,.6,1.5]:
            for yaw in [0,45,180,225]:
                ops=[op('attribute',bits(step),name='minecraft:step_height'),op('yaw',single(yaw))]
                ops += ticks(33,35)+ticks(37,15)+ticks(97,10)+ticks(49,10)+ticks(1,15)+ticks(0,5)
                yield dict(position=vec(start),ops=ops)
    for material in ['stone','slime_block','honey_block','hay_block','farmland','turtle_egg']:
        for shift in [0,32]:
            ops=[op('block',name='minecraft:'+material,position=[25,3,25])]+ticks(shift,80)
            ops += [op('block',name='minecraft:stone',position=[25,3,25])]
            yield dict(position=vec((25.5,10,25.5)),ops=ops)
    for start in [(1.5,4,22.5),(-7.5,4,22.5),(25.5,4,25.5)]:
        ops=ticks(33,10)+ticks(32,10)+ticks(0,10)+ticks(49,10)+ticks(0,10)
        yield dict(position=vec(start),ops=ops)

def corpus():
    yield from attribute_corpus(False)
    yield from pose_corpus(False)
    def food(level=20,saturation=5,exhaustion=0,timer=0):
        return op('food',food=level,saturation=single(saturation),exhaustion=single(exhaustion),timer=timer)
    # Actual ServerPlayer.checkMovementStatistics with all query contexts. These
    # overrides apply only to direct statistics calls, never to resident physics.
    for flags in range(512):
        for invulnerable in [False,True]:
            ops=[food(exhaustion=-0.)]
            for delta in [(0,0,0),(-0.,-0.,-0.),(.005,.005,0),(.125,.75,-.25),(1,2,3)]:
                ops.append(op('statistics_context',vec(delta),flags=flags,invulnerable=invulnerable))
            ops.append(op('food_tick'))
            yield dict(position=vec((25,4,25)),ops=ops)
    values=[-float('inf'),-1e30,-1,-.005,-0.,0.,1e-300,.004999999999999999,.005,.005000000000000001,
            .25499999999999995,.255,.25500000000000006,1,65536,167772.155,21474836.475,21474836.48,
            42949672.96,92233720368547760.,1e30,float('inf'),float('nan')]
    for flags in [0,2,4,8,16,32,96,160,256]:
        for value in values:
            for axis in range(3):
                delta=[0.,0.,0.];delta[axis]=value
                ops=[food(exhaustion=-0.),op('statistics_context',vec(delta),flags=flags,invulnerable=False),op('food_tick')]
                yield dict(position=vec((25,4,25)),ops=ops)
    rng=random.Random(0x53544154)
    for _ in range(1000):
        flags=rng.choice([0,2,4,8,16,32,96,160,256])
        delta=[rng.uniform(-1e5,1e5) if rng.randrange(2) else rng.choice(values) for _ in range(3)]
        yield dict(position=vec((25,4,25)),ops=[food(exhaustion=rng.choice([0,4,39.99,float('nan')])),
            op('statistics_context',vec(delta),flags=flags,invulnerable=rng.choice([False,True]))])
    for sprint in [False,True]:
        for level in [0,6,7,18,20]:
            for health in [10,20]:
                ops=[op('sprint',sprint),food(level),op('health',single(health))]
                for i in range(360):
                    if i%20==0:ops.append(op('yaw',single(0 if (i//20)%2==0 else 180)))
                    ops.append(op('simulation_tick',vec((0,0,.98)),jumping=100<=i<160))
                yield dict(position=vec((12,4,0)),ops=ops)
    # Direct dry statistics queries still use actual registry-backed climbing.
    for start in [(25,4,25),(1,4,22),(-7,4,22),(25,8,25)]:
        for sprint in [False,True]:
            yield dict(position=vec(start),ops=[op('sprint',sprint),op('move',vec((0,-.1,0))),
                *[op('movement_statistics',vec(delta)) for delta in [(0,0,0),(0,.004,0),(0,.5,0),(.01,0,0),(.5,0,.5),(0,-1,0)]]])
    # Hunger, healing and registered starvation share the exact resident state.
    for difficulty in range(4):
        for regenerate in [False,True]:
            for level,saturation in [(20,5),(20,0),(19,0),(18,0),(17,0),(0,0)]:
                for health in [1,9,10,11,19,20]:
                    ops=[op('rules',difficulty=difficulty,regenerate=regenerate),food(level,saturation),op('health',single(health))]
                    ops += [op('clock') if i%2==0 else op('food_tick') for i in range(340)]
                    yield dict(position=vec((25,4,25)),ops=ops)
    for difficulty in range(4):
        for absorption in [0,1,4]:
            for health in [0,1,2,10,11,20]:
                ops=[op('rules',difficulty=difficulty,regenerate=False),op('attribute',bits(20),name='minecraft:max_absorption'),
                     op('absorption',single(absorption)),food(0,0,timer=79),op('health',single(health)),op('food_tick'),op('food_tick')]
                yield dict(position=vec((25,4,25)),ops=ops)
    for exhaustion in [0,4,4.000000476837158,5,40,float('inf'),float('nan')]:
        for saturation in [-1,-0.,0.,.5,1,5,float('nan')]:
            for timer in [-2147483648,-1,0,8,9,78,79,2147483647]:
                yield dict(position=vec((25,4,25)),ops=[op('health',single(10)),food(20,saturation,exhaustion,timer),
                    op('food_tick'),op('food_tick'),op('exhaust',single(5)),op('eat',single(.6),nutrition=4),op('food_tick')])
    for count in [-2147483648,-20,-10,-1,0,1,9,10,19,20,21,2147483647]:
        for health in [0,10,20]:
            for saturation in [0,19,19.5,20,float('nan')]:
                yield dict(position=vec((25,4,25)),ops=[op('rules',difficulty=0,regenerate=True),op('tick_count',count),
                    op('health',single(health)),food(19,saturation),op('peaceful'),op('clock'),op('peaceful')])
    for difficulty in range(4):
        for regenerate in [False,True]:
            ops=[op('rules',difficulty=difficulty,regenerate=regenerate)]
            ops += [op('simulation_tick',vec((0,0,0)),jumping=80<=i<110) for i in range(200)]
            yield dict(position=vec((25,18,25)),ops=ops)
    for difficulty in range(4):
        ops=[op('rules',difficulty=difficulty,regenerate=False),food(0,0),op('health',single(3))]
        ops += [op('simulation_tick',vec((0,0,.98 if i<60 else 0)),jumping=i%30<15) for i in range(330)]
        yield dict(position=vec((25,4,25)),ops=ops)
    for timer in [8,9,78,79]:
        for maximum in [5,20,40]:
            ops=[op('attribute',bits(maximum),name='minecraft:max_health'),op('health',single(4)),food(20,5,0,timer)]
            for regenerate in [True,False,True,True]:
                ops += [op('rules',difficulty=2,regenerate=regenerate),op('food_tick')]
            ops += [food(0,0,0,79),op('rules',difficulty=3,regenerate=False),op('food_tick'),op('rules',difficulty=0,regenerate=True),op('simulation_tick',vec((0,0,0)),jumping=False)]
            yield dict(position=vec((25,4,25)),ops=ops)
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

def sprint_corpus():
    yield from attribute_corpus(True)
    yield from pose_corpus(True)
    for level in [0,6,7,8,20]:
        for health in [10,20]:
            for start in [(12,4,0),(25,18,25)]:
                ops=[op('food',food=level,saturation=single(0),exhaustion=single(0),timer=0),op('health',single(health))]
                for i in range(400):
                    if i%20==0:ops.append(op('yaw',single(0 if i//20%2==0 else 180)))
                    keys=65 if i<20 or i>=160 else 0 if i in [80,81,83,120,121,122] else 1
                    if 200<=i<250:keys|=16
                    ops.append(op('controlled_tick',keys))
                yield dict(position=vec(start),ops=ops)
    for keys in range(128):
        for level in [6,7,20]:
            ops=[op('food',food=level,saturation=single(0),exhaustion=single(3.99),timer=0)]
            ops += [op('controlled_tick',k) for k in [keys,keys,0,1,0,1,65,1,0]]
            yield dict(position=vec((12,4,0)),ops=ops)
    for wall in ['stone','slime_block','honey_block']:
        ops=[op('block',name='minecraft:'+wall,position=[12,4,3])]
        for i in range(140):
            if i==70:ops.append(op('yaw',single(180)))
            ops.append(op('controlled_tick',65))
        ops.append(op('block',name='minecraft:air',position=[12,4,3]))
        yield dict(position=vec((12.5,4,0)),ops=ops)
    for gap in range(12):
        seq=[0,1,0]+[0]*gap+[1]*12+[0]*3+[65]*20+[1]*12+[0]
        yield dict(position=vec((12,4,0)),ops=[op('controlled_tick',k) for k in seq])
    # Sprint-jumping: Player.jumpFromGround's forward impulse while sprinting,
    # with sprint held (65) or started by double-tapping forward, and jump
    # pressed with it, tapped, held through landings (bunny hops) or held
    # before sprinting; at several headings, with too little food to sprint,
    # under a two-block ceiling and up onto a slab.
    runs={
        'held':[65]*12+[81]+[65]*14+[81]*2+[65]*14+[0]*10,
        'together':[0,81]+[81]*30+[0]*10,
        'hop':[65]*8+[81]*40+[0]*10,
        'jump_first':[16]*3+[17]*3+[81]*30+[0]*10,
        'double_tap':[1,0,1]+[1]*12+[17]+[1]*14+[17]*25+[0]*10,
        'release_sprint':[65]*10+[81]*6+[17]*20+[1]*10+[0]*5,
    }
    for name,seq in runs.items():
        for yaw in [0,45,90,180,300]:
            for food in [20,6]:
                for scene in ['open','ceiling','slab']:
                    ops=[op('food',food=food,saturation=single(5),exhaustion=single(0),timer=0),op('yaw',single(yaw))]
                    blocks=[]
                    if scene=='ceiling':blocks=[([8,6,8+i],'stone',{}) for i in range(14)]
                    elif scene=='slab':blocks=[([8,4,12],'stone_slab',{'type':'bottom'}),([8,4,13],'stone',{})]
                    ops += [op('block',name='minecraft:'+b,position=pos,properties=props) for pos,b,props in blocks]
                    ops += [op('controlled_tick',k) for k in seq]
                    ops += [op('block',name='minecraft:air',position=pos) for pos,_,_ in blocks]
                    yield dict(position=vec((8.5,4,8.5)),ops=ops)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--sprint-input',action='store_true');args=parser.parse_args()
    lock,jars,libraries=base.verify_artifacts()
    if args.sprint_input:
        try:from . import test_model_baking_parity as baking
        except ImportError:import test_model_baking_parity as baking
        libraries=baking.client_libraries();jars={'client':jars['client']}
    build=lab.LOCAL/('rust-native-sprint-scene' if args.sprint_input else 'rust-native-scene-motion');build.mkdir(exist_ok=True)
    report=build/'report.json';report.unlink(missing_ok=True)
    resources=lab.LOCAL/'horizon-client'/lock['minecraft']['version']
    resource_hash=client.resource_fingerprint(resources)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSceneMotionOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaSceneCollisionOracle','VanillaRegistryCollisionScanOracle',
        'VanillaHeightmapOracle','VanillaCollisionOracle','VanillaBlockStateCodecOracle','VanillaSuffocationOracle',
        'VanillaFaceSupportOracle','VanillaEntityAttributeOracle','VanillaBlockLandingOracle','VanillaMovementRegistryOracle','VanillaDamageSourceOracle']]
    if args.sprint_input:helpers.append(oracle.with_name('VanillaSprintInputOracle.java'))
    sources=[Path(__file__),lab.ROOT/'tools/test_native_client.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',
             lab.ROOT/'tools/prepare_native_client.py',oracle,*helpers,lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',
             lab.ROOT/'crates/horizon-client/examples/scene_motion_probe.rs']
    for crate in ['horizon-core','horizon-world','horizon-assets','horizon-client']:
        sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    if args.sprint_input:sources.append(lab.ROOT/'tools/test_model_baking_parity.py')
    tested=base.source_hashes(sources)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:
            subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    dependencies=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-cp',str(jars.get('server',jars['client']))+os.pathsep+dependencies,
         '-d',classes,oracle,*helpers],'compile-java')
    fixtures=list(sprint_corpus() if args.sprint_input else corpus());operations=sum(len(f['ops']) for f in fixtures);expected_lines=7*operations+len(fixtures)
    (build/'inputs.txt').write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures))
    input_hash=lab.digest(build/'inputs.txt','sha256')
    damage_dir=build/'damage';damage_dir.mkdir(exist_ok=True)
    for target, source in [('types.json','damage-types.json'),('tags.json','damage-tags.json')]:
        (damage_dir/target).write_bytes((resources/source).read_bytes())
    run(['cargo','build','--locked','-p','horizon-client','--example','scene_motion_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT))
    binary=Path(metadata['target_directory'])/'debug/examples'/('scene_motion_probe.exe' if os.name=='nt' else 'scene_motion_probe')
    run([binary,resources,build,'--describe'],'describe')
    references=[]
    for side,jar in jars.items():
        run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),
             'dev.horizonlab.parity.VanillaSceneMotionOracle',side,build,resources],side)
        replay_hash=lab.digest(build/('inputs-'+side+'.txt'),'sha256')
        run([binary,resources,build,side],'rust-'+side)
        if replay_hash!=lab.digest(build/('inputs-'+side+'.txt'),'sha256'):raise AssertionError('Identity replay changed')
        matched=base.compare(build/(side+'.txt'),build/('rust-'+side+'.txt'))
        if matched!=expected_lines:raise AssertionError('Incomplete motion output')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched,'identity_input_sha256':replay_hash,'rust_output_sha256':lab.digest(build/('rust-'+side+'.txt'),'sha256')})
        print(f'{side}: {matched:,} evolving scene motion states match',flush=True)
    base.ensure_sources_unchanged(sources,tested)
    if resource_hash!=client.resource_fingerprint(resources) or input_hash!=lab.digest(build/'inputs.txt','sha256'):
        raise AssertionError('Motion inputs changed during verification')
    lab.write_json(report,{'schema':2,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],
        'platform':platform.platform(),'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
        'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'fixtures':len(fixtures),
        'operations':operations,'sources_sha256':tested,'resources':resource_hash,'input_sha256':input_hash,
        'references':references,
        'scope':('Client sprint scene executes the real LocalPlayer.aiStep prefix through sprint decisions, stopping at the riding/parent movement boundary, followed by real LocalPlayer.modifyInput and the native server actor simulation. Sprint setters dispatch to that server actor, sharing its actual FoodData and collision conditions. Compares local sprint timer/input history and actual attribute changes, movement, distance exhaustion and hunger. Actual local crouch selection precedes new keyboard input; slowdown, sprint eligibility and pose-dependent dimensions share resident state. No effects, equipment, vehicles or fluid physics. Full client/server tick and networking remain unfinished. ' if args.sprint_input else '')+'Stored PlayerMotion binds resident client chunks and registered attributes/coefficients to air travel, jumping, Entity.move, step resolution, ground/support updates, restitution and speed factors. Repeated native Player.travel/jump/move and actual Player.aiStep / direct LivingEntity.aiStep compare position, velocity, bounds, collision/ground flags, support, climb state, fall-reset field, both sync flags, speed, head yaw, jump trigger and jump delay by exact bits. Prepared-local-input Player.aiStep paths verify the stored speed refresh after movement, with empty inventory and no nearby entities. Travel uses the actual Player.getSpeed movement-attribute query, independently of that stored base-living speed field; both getter and setter execute native bodies. Dry aiStep includes small-velocity cleanup, input damping, held/released jump timing and server frost-modifier removal. Snapshots also compare all allocated attribute instances, base/cache/dirty state, modifier values/operations/permanence and pending update/sync membership. Per-reference JVM holder/instance hashes are replayed so pending callback order and resulting ordered events also match. Dry resident scene with standing, crouching and crawling bounds, cached eye height, native pose-fit selection and ledge backoff; zero frozen ticks, empty equipment, no interpolation, fluids, vehicles, effects or other entities. The oracle controls pose storage and invokes actual native refreshDimensions on changes; full synchronized pose schema and network notifications remain separate. Actual pending attribute refresh runs after player movement and before pose selection in simulation operations, clamping health/absorption and rebuilding scale-dependent dimensions. Native ServerPlayer waypoint-receive and inherited transmit callbacks are compared as ordered requests; the waypoint manager and its network delivery remain controlled consumers. Direct refresh operations and the full enclosing tick remain separate boundaries. Actual LivingEntity.checkFallDamage, registered Block.fallOn/bounceOn and Player/LivingEntity.causeFallDamage are composed, including float-narrowed fall-distance accumulation, reset, last-climb clearing, safe-fall and damage-multiplier attributes, impulse exemptions and ordered landing particle/sound/statistic/hit-ground/damage requests. Farmland pushes the stored player and replaces actual chunk state; repeated turtle-egg changes preserve hatch. Block state at the landing fixture is compared after every operation. World RNG is a seeded native legacy stream. Actual ServerPlayer/Player/LivingEntity hurt dispatch now updates native synchronized health and player absorption, modified armor/toughness reduction, exhaustion, damage cooldown/last-source state and velocity synchronization. Native health storage and dirty flags are compared after every operation. Damage sources use actual decoded/tagged pinned damage registries. ServerPlayer jump statistics and exhaustion are included; dead-or-dying state disables movement input and jumping. Empty equipment/hands/effects, configurable difficulty and natural regeneration, damage gamerules enabled, loaded client and no spawn-extra fall particles define this scene domain. Ordered health/statistic/combat-record/broadcast/sound/criterion/death requests are compared; criterion argument payloads retain the separate combat gate. Actual health changes are applied, but audiovisual playback, persistent statistics, combat tracker internals, full death/drop/respawn services, block neighbor/drop/block-entity callbacks, movement emission, block-inside and networking remain controlled consumers. Actual FoodData.tick and ServerPlayer.tickRegeneration now share the same stored health, attributes, exhaustion and registered starvation damage. Snapshots compare food level, saturation, exhaustion, timer and tick count. Cases cover all four difficulties, regeneration rule changes, absorption during starvation, Peaceful recovery, timer overflow and nonfinite food values. Simulation operations compose clock setup, actual player movement and actual food ticking in their native order, including fall damage followed by regeneration in the same tick. Explicit clock operations establish cooldown/hurt-time/game-time/tick-count state; full enclosing native tick/doTick methods are not invoked by those setup operations. Input preparation has its own client comparison. ServerPlayer.checkMovementStatistics executes on accepted resident deltas after movement and before food ticking. Additional direct method fixtures cover all 512 overlapping passenger/swim/eye-water/water/climb/ground/sprint/crouch/glide flag combinations, invulnerable abilities, centimeter rounding, overflow, signed zero and nonfinite deltas. Those query contexts apply only to the statistics method; fluid/vehicle/glide physics remain unbound. ServerGamePacketListener packet acceptance/rejection and scheduling are not executed. No full Player.tick, consumable item-use lifecycle, local input lifecycle, nonempty inventory/equipment ticks, full freezing, complete death lifecycle or full gameplay parity claim.'})
    print(f'Scoped scene motion parity passed: {report}',flush=True)

if __name__=='__main__':main()
