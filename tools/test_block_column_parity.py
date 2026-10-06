#!/usr/bin/env python3
"""Compare native block-column sampling, obstruction, truncation and writes."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    encode=lambda value:json.dumps(value,separators=(',',':'))
    directions={'down':(0,-1,0),'up':(0,1,0),'north':(0,0,-1),'south':(0,0,1),'west':(-1,0,0),'east':(1,0,0)}
    def feature(heights=(2,3,1),providers=({'id':'stone'},{'id':'water'},{'id':'pale_moss_carpet'}),direction='up',allowed=None,tip=False):
        return {'type':'block_column','layers':[{'height':height,'provider':providers[i%len(providers)]} for i,height in enumerate(heights)],'direction':direction,'allowed_placement':allowed if allowed is not None else {'type':'true'},'prioritize_tip':tip}
    def case(definition,**extra):
        f={'definition':encode(definition),'mode':'feature','origin':[0,64,0],'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,'brightness':15,'fallback':get('air'),'states':[],'repeats':2};f.update(extra);return f
    for state in states:yield case(feature((1,),[state]))
    heights=[[],[0],[1],[0,0],[0,2,0,1],[2,3,1],[{'type':'uniform','min_inclusive':0,'max_inclusive':3},1,{'type':'weighted_list','distribution':[{'data':0,'weight':1},{'data':3,'weight':2}]}]]
    # The first allowed-placement test is one step beyond the origin. Truncation
    # chooses base or tip layers before any provider is sampled or write occurs.
    for direction,step in directions.items():
        for selected in heights:
            for tip in [False,True]:
                for blocked in range(9):
                    for legacy in [False,True]:
                        for seed in [0,-1,0x123456789abcdef]:
                            for fail in [-1,0,2,5,9]:
                                obstruction=[step[0]*blocked,64+step[1]*blocked,step[2]*blocked,get('stone')]
                                yield case(feature(selected,direction=direction,allowed={'type':'matching_blocks','blocks':['air']},tip=tip),states=[obstruction],legacy=legacy,seed=seed,fail=fail,reject=3 if seed else 0)
    providers=[{'type':'random_block','blocks':[]},{'type':'random_block','blocks':['stone','water','pale_moss_carpet']},{'type':'rotated','state':{'id':'oak_log'}},{'type':'weighted','entries':[{'data':'stone','weight':1},{'data':'water','weight':2}]},'horizon:stone',{'type':'rule_based','rules':[]}]
    for provider,direction,tip in itertools.product(providers,directions,[False,True]):
        for legacy,seed,fail,reject in itertools.product([False,True],[0,-1,0x123456789abcdef],[-1,0,1,4],[0,1,3]):
            yield case(feature([{'type':'uniform','min_inclusive':0,'max_inclusive':3},2,1],[provider],direction=direction,tip=tip),legacy=legacy,seed=seed,fail=fail,reject=reject,repeats=3)
    predicates=[{'type':'true'},{'type':'not','predicate':{'type':'true'}},{'type':'solid'},{'type':'replaceable'},{'type':'has_sturdy_face','direction':'up'},{'type':'would_survive','state':'small_dripleaf'},{'type':'all_of','predicates':[{'type':'replaceable'},{'type':'would_survive','state':'pale_moss_carpet'}]}]
    for predicate in predicates:
        for neighbor in ['air','stone','water','clay','shulker_box','moving_piston','scaffolding']:
            for fail in [-1,0,1,2,3,6]:yield case(feature(allowed=predicate),fallback=get(neighbor),fail=fail)
    for origin in [[2147483647,2147483647,2147483647],[-2147483648,-2147483648,-2147483648],[-17,-64,-33]]:
        for direction in directions:yield case(feature(direction=direction),origin=origin)
    for key,values in [('layers',[None,False,[],[{}],[{'height':-1,'provider':{'id':'stone'}}],[{'height':1,'provider':None}],[{'height':1,'provider':'missing'}]]),('direction',[None,False,0,'UP','missing']),('allowed_placement',[None,{},True]),('prioritize_tip',[None,0,1,'true',False,True])]:
        for value in values:
            definition=feature();definition[key]=value;yield case(definition)
        definition=feature();del definition[key];yield case(definition)
    for provider in [-1,{'type':'uniform','min_inclusive':-1,'max_inclusive':2},{'type':'uniform','min_inclusive':0,'max_inclusive':2147483647},2147483647]:
        # A false first obstruction permits huge positive heights to truncate
        # without an enormous world traversal.
        yield case(feature([provider],allowed={'type':'not','predicate':{'type':'true'}}))
    for tip in [False,True]:
        column=feature(tip=tip,allowed={'type':'matching_blocks','blocks':'air'})
        placed={'feature':column,'placement':[{'type':'count','count':2},{'type':'offset','x':1,'y':0,'z':-1}]}
        for legacy,seed,fail,reject in itertools.product([False,True],[0,-1],[-1,0,3,8],[0,1,3]):
            yield case(placed,mode='placed',legacy=legacy,seed=seed,fail=fail,reject=reject)
            yield case({'type':'sequence','features':[placed,{'feature':'horizon:moss','placement':[]}]},legacy=legacy,seed=seed,fail=fail,reject=reject)


if __name__=='__main__':
    harness.run(corpus,'rust-block-column-parity',__file__,
        'Native block-column feature codec and algorithms, with nonnegative height-provider validation, all candidate state descriptors, all six directions, both base/tip truncation orders, first obstruction beyond the origin, ordered predicate queries before writes, optional-provider fallback reads, rejected writes, partial failures, coordinate wrapping, named providers and nested feature/modifier composition. Both independent random streams and stored states are checked after repeated calls. A bounded runtime work budget rejects pathological million-operation traversals; actual chunks, neighbor updates and complete gameplay remain pending.')
