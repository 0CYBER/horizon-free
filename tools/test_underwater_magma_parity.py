#!/usr/bin/env python3
"""Compare underwater magma scans, face occlusion and randomized placement."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    def magma(**extra):
        value={'type':'underwater_magma','floor_search_range':2,'placement_radius_around_floor':0,'placement_probability_per_valid_position':1.0};value.update(extra);return value
    def case(definition=None, **extra):
        origin=extra.pop('origin',[0,64,0])
        value={'definition':json.dumps(definition if definition is not None else magma(),separators=(',',':')),
               'mode':'feature','origin':origin,'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,
               'brightness':15,'fallback':get('stone'),'states':[[*origin,get('water')]],'repeats':1}
        value.update(extra);return value
    for state in range(len(states)):
        for position in [[0,64,0],[0,63,0],[0,62,0],[0,63,-1],[1,63,0],[0,63,1],[-1,63,0]]:
            initial={(0,64,0):get('water')};initial[tuple(position)]=state
            yield case(states=[[*p,s] for p,s in initial.items()])
    for search,above,below,legacy,fail in itertools.product([0,1,2,3,4,8,512],range(4),range(4),[False,True],[-1,0,1,2,3,5,9]):
        initial=[[0,64+y,0,get('water')] for y in range(-below,above+1)]
        yield case(magma(floor_search_range=search),states=initial,legacy=legacy,fail=fail)
    for radius,probability,legacy,seed,fail,reject in itertools.product([0,1,2],[0.0,0.1,0.5,1.0],[False,True],
            [0,-1,0x123456789abcdef],[-1,0,2,6,8,12,19],[0,1,3]):
        yield case(magma(placement_radius_around_floor=radius,placement_probability_per_valid_position=probability),
                   legacy=legacy,seed=seed,fail=fail,reject=reject,repeats=2)
    for legacy in [False,True]:
        yield case(magma(placement_radius_around_floor=64,placement_probability_per_valid_position=0.0),legacy=legacy)
    for origin,search in itertools.product([[2147483647,2147483647,2147483647],[-2147483648,-2147483648,-2147483648],[-17,-64,-33]],[0,1,2,3,512]):
        yield case(magma(floor_search_range=search),origin=origin)
    for key,values in [('floor_search_range',[None,-1,0,1,512,513,False,'2',4294967298]),
                       ('placement_radius_around_floor',[None,-1,0,64,65,False,'0']),
                       ('placement_probability_per_valid_position',[None,-1,-0.0,-1e-100,0,1e-100,1,1.0000000001,1.01,False,'1'])]:
        for value in values:
            definition=magma(placement_probability_per_valid_position=0.0);definition[key]=value;yield case(definition)
        definition=magma();del definition[key];yield case(definition)
    for legacy,seed,fail,reject in itertools.product([False,True],[0,-1],[-1,0,5,12],[0,3]):
        placed={'feature':magma(placement_radius_around_floor=1,placement_probability_per_valid_position=0.5),'placement':[{'type':'count','count':2}]}
        yield case(placed,mode='placed',legacy=legacy,seed=seed,fail=fail,reject=reject)
        yield case({'type':'overlay','features':[placed,{'feature':'horizon:moss','placement':[]}]},legacy=legacy,seed=seed,fail=fail,reject=reject)


if __name__=='__main__':
    harness.run(corpus,'rust-underwater-magma-parity',__file__,
        'Native UnderwaterMagmaFeature codecs and Column.scan floor selection. Initial water identity, upward scan before downward scan, repeated boundary reads, all scan endpoints, every state at origin/floor/below and each horizontal neighbor, exact full-face occlusion, cubic traversal, per-position float draws before block tests, flags 2, rejected writes and partial failures are compared. Both random modes include the complete radius-64 cube with probability zero (2146689 draws), wrapped radius-zero columns and nested placement. Rust allows the entire valid codec radius domain and bounds pathological overflow iteration at 2^23 visits. Actual chunk update callbacks, magma gameplay and full world generation remain pending.')
