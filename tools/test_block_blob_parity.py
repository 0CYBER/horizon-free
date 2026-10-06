#!/usr/bin/env python3
"""Compare block-blob descent, three randomized shapes and state mutations."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    def blob(state=None, **extra):
        value={'type':'block_blob','state':state if state is not None else {'id':'mossy_cobblestone'},'can_place_on':{'type':'matching_blocks','blocks':'stone'}}
        value.update(extra);return value
    def case(definition=None, **extra):
        value={'definition':json.dumps(definition if definition is not None else blob(),separators=(',',':')),
               'mode':'feature','origin':[0,64,0],'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,
               'brightness':15,'fallback':get('stone'),'states':[],'repeats':2}
        value.update(extra);return value
    for state in states:yield case(blob(state))
    for state in range(len(states)):
        yield case(origin=[0,-60,0],states=[[0,-61,0,state]],repeats=1)
    predicates=[{'type':'true'},{'type':'not','predicate':{'type':'true'}},
                {'type':'matching_blocks','blocks':['stone','water']},
                {'type':'matching_blocks','blocks':'#minecraft:logs'},
                {'type':'has_sturdy_face','direction':'up'},
                {'type':'would_survive','state':'short_grass'}, {'type':'solid'}]
    for predicate,legacy,seed,fail,reject in itertools.product(predicates,[False,True],range(24),[-1,0,1,3,9,19],[0,1,3]):
        yield case(blob(can_place_on=predicate),legacy=legacy,seed=seed,fail=fail,reject=reject,
                   origin=[15,-48,15],states=[[15,-49,15,get('air')],[15,-50,15,get('oak_log')]])
    for height,depth in itertools.product([-64,-62,-61,-60,-59,0,319,320],range(8)):
        initial=[[0,height-1-offset,0,get('air')] for offset in range(depth)]
        yield case(origin=[0,height,0],states=initial)
    for legacy,seed,origin in itertools.product([False,True],[0,-1,0x123456789abcdef],
            [[-2147483640,64,-2147483640],[2147483640,64,2147483640],[-17,-2147483648,-33]]):
        yield case(legacy=legacy,seed=seed,origin=origin)
    for key,values in [('state',[None,False,{},[],{'id':'missing'},'missing',{'id':'oak_log','properties':{'axis':'invalid'}}]),
                       ('can_place_on',[None,False,{},[],{'type':'missing'}])]:
        for value in values:
            definition=blob();definition[key]=value;yield case(definition)
        definition=blob();del definition[key];yield case(definition)
    for legacy,seed,fail,reject in itertools.product([False,True],[0,-1,0x123456789abcdef],[-1,0,5,17],[0,3]):
        placed={'feature':blob(),'placement':[{'type':'count','count':2},{'type':'in_square'}]}
        yield case(placed,mode='placed',legacy=legacy,seed=seed,fail=fail,reject=reject)
        yield case({'type':'overlay','features':[placed,{'feature':'horizon:moss','placement':[]}]},legacy=legacy,seed=seed,fail=fail,reject=reject)


if __name__=='__main__':
    harness.run(corpus,'rust-block-blob-parity',__file__,
        'Native BlockBlobFeature codec, state decoding, support-predicate descent and three randomized overlapping shapes. Every candidate and supporting registry state, minimum-height returns, query failures, float radius versus double distance, inclusive box order, all three final origin-offset draws, flags 3, rejected writes, repeated mutations and both random modes are compared. Named tags, survival/support predicates and overlay/modifier composition use the same mutable world. Positions near signed integer limits avoid pathological native overflow boxes; Rust bounds descent and box work to 2^20 visits. Resident-world update callbacks and complete gameplay remain pending.')
