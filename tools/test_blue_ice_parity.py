#!/usr/bin/env python3
"""Compare blue-ice sea-level gating, seeding and growth attempts."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    directions=[(0,-1,0),(0,1,0),(0,0,-1),(0,0,1),(-1,0,0),(1,0,0)]
    def case(**extra):
        origin=extra.pop('origin',[0,60,0]);wrap=lambda n:(n+2**31)%2**32-2**31
        value={'definition':'{"type":"blue_ice"}','mode':'feature','origin':origin,'legacy':False,'seed':0,'world_seed':1,
               'fail':-1,'reject':0,'brightness':15,'sea':63,'fallback':get('air'),
               'states':[[*origin,get('water')],[origin[0],wrap(origin[1]+1),origin[2],get('packed_ice')]],'repeats':1}
        value.update(extra);return value
    for state in range(len(states)):
        yield case(states=[[0,60,0,state],[0,59,0,get('water')],[0,61,0,get('packed_ice')]],fail=24)
        yield case(fallback=state,fail=24)
    for mask,legacy,seed,fail,reject in itertools.product(range(64),[False,True],[0,-1,0x123456789abcdef],[-1,0,1,3,6,9,21],[0,1,3]):
        initial=[[0,60,0,get('water')],*[[x,60+y,z,get('packed_ice') if mask&(1<<i) else get('air')] for i,(x,y,z) in enumerate(directions)]]
        yield case(states=initial,legacy=legacy,seed=seed,fail=fail,reject=reject,repeats=2)
    for name,legacy,seed in itertools.product(['air','cave_air','void_air','water','packed_ice','ice','blue_ice','stone','lava','oak_slab'],[False,True],range(16)):
        yield case(fallback=get(name),legacy=legacy,seed=seed,repeats=2)
    for y,sea,legacy in itertools.product([-2147483648,-64,61,62,63,64,2147483647],[-2147483648,-64,0,63,2147483647],[False,True]):
        yield case(origin=[-17,y,-33],sea=sea,legacy=legacy)
    for origin in [[2147483647,-2147483648,2147483647],[-2147483648,-2147483648,-2147483648]]:
        for legacy in [False,True]:yield case(origin=origin,legacy=legacy)
    for legacy,fail,reject in itertools.product([False,True],[-1,0,3,8,20],[0,3]):
        placed={'feature':{'type':'blue_ice','ignored':None},'placement':[{'type':'count','count':2}]}
        yield case(definition=json.dumps(placed),mode='placed',legacy=legacy,fail=fail,reject=reject,repeats=2)
        yield case(definition=json.dumps({'type':'overlay','features':[placed,{'feature':'horizon:moss','placement':[]}]}),legacy=legacy,fail=fail,reject=reject)


if __name__=='__main__':
    harness.run(corpus,'rust-blue-ice-parity',__file__,
        'Native BlueIceFeature unit codec, world sea-level gate, origin/below water checks, first non-down packed-ice neighbor and seeding. Every origin and candidate-state classification is compared through failure-terminated prefixes; complete 200-attempt runs cover all packed-ice neighbor masks, ten representative fill states, both random streams and many seeds. Vertical bound draws, signed division, X/Z draw order, six-direction blue-ice adjacency, mutable spreading, flags 2, rejected writes, partial failures, extreme sea levels, wrapped positions and overlay/modifier composition are included. Chunk updates, ice gameplay and full world generation remain pending.')
