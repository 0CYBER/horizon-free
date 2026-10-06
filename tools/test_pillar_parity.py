#!/usr/bin/env python3
"""Compare vertical pillars, cap placement and early-exit random consumption."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts,placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    def pillar(block=None,**extra):
        definition={'type':'single_block_pillar','block':block if block is not None else {'id':'stone'},'direction':'up'};definition.update(extra);return definition
    def case(definition,**extra):
        f={'definition':json.dumps(definition,separators=(',',':')),'mode':'feature','origin':[0,318,0],'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,'brightness':15,'fallback':get('air'),'states':[],'repeats':2};f.update(extra);return f
    for state in states:yield case(pillar(state))
    providers=[{'id':'stone'},{'id':'water'},{'type':'random_block','blocks':[]},{'type':'random_block','blocks':['stone','pale_moss_carpet']},{'type':'rotated','state':{'id':'oak_log'}},'horizon:stone']
    caps=[None,'horizon:moss','horizon:empty',{'feature':{'type':'no_op'},'placement':[]},{'feature':'horizon:stone','placement':[{'type':'count','count':2}]},{'feature':{'type':'block_column','layers':[{'height':2,'provider':{'id':'water'}}],'direction':'down','allowed_placement':{'type':'true'},'prioritize_tip':True},'placement':[]}]
    for direction,chance,cap in itertools.product(['up','down'],[0.0,0.000001,0.25,0.5,0.999999,1.0],caps):
        for legacy,seed,fail,reject in itertools.product([False,True],[0,-1,0x123456789abcdef],[-1,0,2,5],[0,1,3]):
            definition=pillar({'type':'random_block','blocks':['stone','water']},direction=direction,chance_to_continue=chance,cap_feature=cap)
            yield case(definition,origin=[0,316 if direction=='up' else -61,0],legacy=legacy,seed=seed,world_seed=seed,fail=fail,reject=reject)
    predicates=[{'type':'true'},{'type':'not','predicate':{'type':'true'}},{'type':'matching_blocks','blocks':'air'},{'type':'would_survive','state':'pale_moss_carpet'},{'type':'has_sturdy_face','direction':'up'},{'type':'replaceable'}]
    for provider,predicate,neighbor,fail in itertools.product(providers,predicates,['air','stone','water','shulker_box'],[-1,0,1,2,5]):
        yield case(pillar(provider,can_replace=predicate,cap_feature='horizon:moss'),fallback=get(neighbor),fail=fail)
    for origin,direction,chance in itertools.product([[0,-65,0],[0,-64,0],[0,319,0],[0,320,0],[2147483647,-2147483648,-2147483648],[-2147483648,2147483647,2147483647]],['up','down'],[0.0,0.5,1.0]):
        yield case(pillar(direction=direction,chance_to_continue=chance,cap_feature='horizon:stone'),origin=origin)
    for key,values in [('block',[None,False,0,{},'missing']),('can_replace',[None,False,0,{}, {'type':'true'}]),('direction',[None,'north','UP',False,0]),('chance_to_continue',[None,False,'0.5',-0.01,1.01,-0.0,-1e-100,1e-100,1.0000000001,0,1]),('cap_feature',[None,'missing',{},False])]:
        for value in values:
            definition=pillar();definition[key]=value;yield case(definition)
        definition=pillar();definition.pop(key,None);yield case(definition)
    nested=pillar(chance_to_continue=0,cap_feature={'feature':pillar(direction='down',chance_to_continue=0.5,cap_feature='horizon:stone'),'placement':[{'type':'count','count':2}]})
    for legacy,seed,fail in itertools.product([False,True],[0,-1,0x123456789abcdef],[-1,0,1,4]):
        yield case(nested,origin=[0,-61,0],legacy=legacy,seed=seed,fail=fail)
        yield case({'type':'sequence','features':[{'feature':nested,'placement':[]},{'feature':'horizon:stone','placement':[]}]},origin=[0,-61,0],legacy=legacy,seed=seed,fail=fail)


if __name__=='__main__':
    harness.run(corpus,'rust-pillar-parity',__file__,
        'Native single-block pillar codec and placement, all provider state descriptors, upward/downward traversal, replacement predicates before continuation draws, bounds checks after draws, cap placement one opposite step from the stopping position even when no column block was placed, ignored rejected writes and cap false results, partial failures, coordinate wrapping, independent world random stream, named holders, nested pillars and feature sequences. Shared graph work limits bound pathological traversals. Actual resident chunks, neighbor updates and complete gameplay remain pending.')
