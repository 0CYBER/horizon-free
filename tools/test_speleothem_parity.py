#!/usr/bin/env python3
"""Compare small speleothem direction, base patches and waterlogged points."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness

FIELDS=['chance_of_taller_generation','chance_of_directional_spread','chance_of_spread_radius2','chance_of_spread_radius3']
def corpus(facts,placement):
    get=lambda name:facts['defaults']['minecraft:'+name]
    def definition(**extra):
        value={'type':'speleothem','base_block':'dripstone_block','pointed_block':'pointed_dripstone','replaceable_blocks':['stone','dirt']}
        value.update(extra);return value
    def case(value=None,**extra):
        f={'definition':json.dumps(definition() if value is None else value),'mode':'feature','origin':[0,64,0],
           'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,'brightness':15,'fallback':get('air'),
           'states':[[0,63,0,get('stone')]],'repeats':1}
        f.update(extra);return f
    zero=definition(**dict.fromkeys(FIELDS,0))
    for state in range(len(facts['states'])):
        yield case(states=[[0,65,0,state]],fail=2)
        yield case(states=[[0,63,0,state]],fail=2)
        yield case(zero,fluid=state)
        yield case(definition(chance_of_taller_generation=1),states=[[0,63,0,get('stone')],[0,65,0,state]],fail=12)
    for legacy,seed,mask,reject in itertools.product([False,True],range(64),range(4),[0,1,3]):
        states=[[0,65,0,get('stone') if mask&1 else get('air')],[0,63,0,get('dirt') if mask&2 else get('air')]]
        yield case(legacy=legacy,seed=seed,states=states,reject=reject,repeats=2)
    for legacy,fail,reject,chance in itertools.product([False,True],[-1,*range(48)],[0,1,3],[0,0.5,1]):
        yield case(definition(**dict.fromkeys(FIELDS,chance)),legacy=legacy,fail=fail,reject=reject,fallback=get('stone'),repeats=2)
    for base,point in itertools.product(['stone',{'id':'oak_log','properties':{'axis':'x'}},'water'],['pointed_dripstone',{'id':'pointed_dripstone','properties':{'vertical_direction':'down','thickness':'base','waterlogged':'true'}},'stone','torch']):
        for legacy in [False,True]:yield case(definition(base_block=base,pointed_block=point,**dict.fromkeys(FIELDS,1)),legacy=legacy,fallback=get('stone'),repeats=2)
    for field in FIELDS:
        for value in [None,-0.0,-1,0,1,1.0000000001,1.1,0.2,'0.5',True,False,[],{}]:
            yield case(definition(**{field:value}))
    for field in ['base_block','pointed_block','replaceable_blocks']:
        value=definition();del value[field];yield case(value)
        for raw in [None,[],{},False,'missing']:
            yield case(definition(**{field:raw}))
    for value in ['#minecraft:dripstone_replaceable_blocks',[],['air','water','stone'],'air','#horizon:missing']:
        yield case(definition(replaceable_blocks=value),repeats=2)
    for origin in [[0,-64,0],[-17,319,-33],[2147483647,2147483647,-2147483648],[-2147483648,-2147483648,2147483647]]:
        below=[origin[0],((origin[1]-1+2**31)%2**32)-2**31,origin[2],get('stone')]
        yield case(definition(**dict.fromkeys(FIELDS,1)),origin=origin,states=[below],repeats=2)
    for legacy,fail,reject in itertools.product([False,True],[-1,0,2,5,19],[0,1,3]):
        placed={'feature':definition(),'placement':[{'type':'count','count':2}]}
        yield case(placed,mode='placed',legacy=legacy,fail=fail,reject=reject)
        yield case({'type':'overlay','features':[placed,{'feature':definition(chance_of_taller_generation=1),'placement':[]}]},legacy=legacy,fail=fail,reject=reject)

if __name__=='__main__':
    harness.run(corpus,'rust-speleothem-parity',__file__,
        'Native SpeleothemFeature codecs and optional float defaults, ceiling/floor base tests, conditional random direction, four horizontal base-patch branches with radius-two/three direction draws, taller-tip chance, post-patch base recheck and waterlogged tip/frustum writes. Base/pointed configured states select block identities and their defaults; invalid pointed properties fail at native placement time. All states cover support/taller/contained-fluid roles, both RNGs, probability boundaries, rejected writes, partial failures, repeated mutation, wrapped coordinates and composition. Base patch updates and point placement are traced in order; neighbor updates, falling behavior, fluids, larger clusters and full cave generation remain pending.')
