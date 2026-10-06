#!/usr/bin/env python3
"""Compare origin placement and exactly-one-neighbor spread behavior."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts,placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    directions=[(0,-1,0),(0,1,0),(0,0,-1),(0,0,1),(-1,0,0),(1,0,0)]
    def spread(block=None,**extra):
        value={'type':'random_neighbor_spread','block':block if block is not None else {'id':'stone'},'accepted_neighbors':'stone','can_replace':{'type':'true'},'attempts':1,'xz_offset':0,'y_offset':0};value.update(extra);return value
    def case(definition,**extra):
        value={'definition':json.dumps(definition,separators=(',',':')),'mode':'feature','origin':[0,64,0],'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,'brightness':15,'fallback':get('air'),'states':[],'repeats':2};value.update(extra);return value
    for state in states:yield case(spread(state))
    for mask,attempts,fail,reject in itertools.product(range(64),[1,2,3],[-1,*range(9)],[0,1,3]):
        initial=[[x,64+y,z,get('stone') if mask&(1<<i) else get('air')] for i,(x,y,z) in enumerate(directions)]
        yield case(spread(attempts=attempts),states=initial,fail=fail,reject=reject)
    for index,state in enumerate(states):
        yield case(spread(accepted_neighbors=['stone','water','oak_log','oak_leaves']),states=[[0,63,0,index]],repeats=1)
    providers=[{'id':'stone'},{'type':'random_block','blocks':[]},{'type':'random_block','blocks':['stone','water']},{'type':'rotated','state':{'id':'oak_log'}},'horizon:stone']
    for provider,accepted,legacy,seed,fail,reject in itertools.product(providers,['stone',[],['air','stone'],'#minecraft:logs','#minecraft:leaves'],[False,True],[0,-1,0x123456789abcdef],[-1,0,1,4,9],[0,1,3]):
        yield case(spread(provider,accepted_neighbors=accepted,attempts={'type':'uniform','min_inclusive':1,'max_inclusive':12},xz_offset={'type':'uniform','min_inclusive':-2,'max_inclusive':2},y_offset={'type':'uniform','min_inclusive':-2,'max_inclusive':2}),legacy=legacy,seed=seed,fail=fail,reject=reject)
    for allowed in [{'type':'not','predicate':{'type':'true'}},{'type':'matching_blocks','blocks':'air'},{'type':'would_survive','state':'pale_moss_carpet'},{'type':'solid'},{'type':'has_sturdy_face','direction':'up'}]:
        for fail in [-1,0,1,2,5]:yield case(spread(can_replace=allowed,attempts=4),fail=fail)
    for origin,offset in itertools.product([[2147483647,2147483647,2147483647],[-2147483648,-2147483648,-2147483648],[-17,-64,-33]],[-16,-1,0,1,16]):
        yield case(spread(xz_offset=offset,y_offset=offset),origin=origin)
    for key,values in [('block',[None,False,{},'missing']),('accepted_neighbors',[None,False,{},[],['missing'],'#missing']),('can_replace',[None,False,{}]),('attempts',[None,0,-1,1,3000,3001,{'type':'uniform','min_inclusive':0,'max_inclusive':2}]),('xz_offset',[None,-17,-16,16,17]),('y_offset',[None,-17,-16,16,17])]:
        for value in values:
            definition=spread();definition[key]=value;yield case(definition)
        definition=spread();del definition[key];yield case(definition)
    for legacy,seed,fail in itertools.product([False,True],[0,-1,0x123456789abcdef],[-1,0,3,8]):
        placed={'feature':spread(attempts=5,xz_offset={'type':'uniform','min_inclusive':-1,'max_inclusive':1}),'placement':[{'type':'count','count':2}]}
        yield case(placed,mode='placed',legacy=legacy,seed=seed,fail=fail)
        yield case({'type':'sequence','features':[placed,{'feature':'horizon:moss','placement':[]}]},legacy=legacy,seed=seed,fail=fail)


if __name__=='__main__':
    harness.run(corpus,'rust-neighbor-spread-parity',__file__,
        'Native random-neighbor-spread codecs and placement. Origin provider evaluation/write precedes attempt sampling and all replacement checks. X/Y/Z offsets retain sample order. Every candidate state and every potential neighbor state, all six-direction acceptance masks, exactly-one acceptance and early exit after a second match, bounded provider codec ranges, empty/named holder sets, optional-provider fallback reads, mutable spread across repeated attempts, rejected writes, partial failures, wrapped positions, both random modes and composed feature/modifier calls are compared. Actual chunks, neighbor callbacks and complete gameplay remain pending.')
