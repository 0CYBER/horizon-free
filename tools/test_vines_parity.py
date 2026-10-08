#!/usr/bin/env python3
"""Compare vine attachment priority, support queries and ordered writes."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    directions=[(0,1,0),(0,0,-1),(0,0,1),(-1,0,0),(1,0,0)]
    def case(**extra):
        value={'definition':'{"type":"vines"}','mode':'feature','origin':[0,64,0],'legacy':False,'seed':0,'world_seed':1,
               'fail':-1,'reject':0,'brightness':15,'fallback':get('air'),'states':[],'repeats':2}
        value.update(extra);return value
    for state in range(len(states)):
        yield case(states=[[0,64,0,state],[0,65,0,get('stone')]])
        for x,y,z in directions:yield case(states=[[x,64+y,z,state]],repeats=1)
    for mask,origin,legacy,seed,fail,reject in itertools.product(range(32),
            [[0,64,0],[2147483647,2147483647,2147483647],[-2147483648,-2147483648,-2147483648]],
            [False,True],[0,-1],[-1,0,1,2,3,4,5,6,7],[0,1,3]):
        wrap=lambda n:(n+2**31)%2**32-2**31
        initial=[[*[wrap(origin[i]+d[i]) for i in range(3)],get('stone') if mask&(1<<index) else get('air')] for index,d in enumerate(directions)]
        yield case(origin=origin,states=initial,legacy=legacy,seed=seed,fail=fail,reject=reject)
    for name in ['air','cave_air','void_air','water','vine']:
        yield case(states=[[0,64,0,get(name)],[0,63,0,get('stone')]])
    for legacy,fail,reject in itertools.product([False,True],[-1,0,3,5],[0,3]):
        placed={'feature':{'type':'vines','ignored':{'bad':None}},'placement':[{'type':'count','count':2}]}
        yield case(definition=json.dumps(placed),mode='placed',legacy=legacy,fail=fail,reject=reject,states=[[0,65,0,get('stone')]])
        yield case(definition=json.dumps({'type':'overlay','features':[placed,{'feature':'horizon:moss','placement':[]}]}),legacy=legacy,fail=fail,reject=reject,states=[[0,65,0,get('stone')]])


if __name__=='__main__':
    harness.run(corpus,'rust-vines-parity',__file__,
        'Native VinesFeature unit codec and placement. Every origin state and every neighbor state in each of the five accepted directions, all support masks, air variants, wrapped coordinates and first-attachment priority Up/North/South/West/East are compared. Native support/collision queries, flags 2, rejected writes, partial failures, repeated calls, unchanged feature/world random streams and overlay/modifier composition share the mutable world. Dynamic block entities are absent in this fixture; resident-world updates, vine growth and complete gameplay remain pending.')
