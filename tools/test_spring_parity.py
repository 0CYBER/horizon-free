#!/usr/bin/env python3
"""Compare spring cavity checks, fluid placement and scheduled fluid ticks."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states=facts['states'];get=lambda name:facts['defaults']['minecraft:'+name]
    directions=[(-1,0,0),(1,0,0),(0,0,-1),(0,0,1),(0,-1,0)]
    def spring(state=None, **extra):
        value={'type':'spring_feature','state':state if state is not None else {'id':'water','properties':{'falling':'false'}},'valid_blocks':'stone'}
        value.update(extra);return value
    def case(definition=None, **extra):
        origin=extra.pop('origin',[0,64,0]);wrap=lambda n:(n+2**31)%2**32-2**31
        position=lambda d:[wrap(origin[i]+d[i]) for i in range(3)]
        initial=[[*position((0,0,0)),get('air')],[*position((-1,0,0)),get('air')]]
        value={'definition':json.dumps(definition if definition is not None else spring(),separators=(',',':')),
               'mode':'feature','origin':origin,'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,
               'brightness':15,'fallback':get('stone'),'states':initial,'repeats':1}
        value.update(extra);return value
    for state in range(len(states)):
        for position in [[0,65,0],[0,63,0],[0,64,0],[1,64,0]]:
            initial={(0,64,0):get('air'),(-1,64,0):get('air')};initial[tuple(position)]=state
            yield case(states=[[*p,s] for p,s in initial.items()])
    palette=[get('air'),get('stone'),get('water')]
    for choices in itertools.product(range(3),repeat=5):
        initial=[[x,64+y,z,palette[choices[i]]] for i,(x,y,z) in enumerate(directions)]+[[0,64,0,get('air')]]
        for required,rocks,holes in itertools.product([False,True],range(6),range(6)):
            yield case(spring(requires_block_below=required,rock_count=rocks,hole_count=holes),states=initial)
        # Air can belong to valid_blocks; rock and hole counts are independent.
        yield case(spring(valid_blocks=['stone','air'],requires_block_below=False,
                          rock_count=sum(v in [0,1] for v in choices),hole_count=choices.count(0)),states=initial)
    fluids=[{'id':'empty'}]
    for name in ['water','lava','flowing_water','flowing_lava']:
        for falling in ['false','true']:
            for level in (range(1,9) if name.startswith('flowing') else [8]):
                fluids.append({'id':name,'properties':{'falling':falling,'level':str(level)}})
    for fluid,legacy,fail,reject,origin in itertools.product(fluids,[False,True],[-1,*range(16)],[0,1,3],
            [[0,64,0],[2147483647,2147483647,2147483647],[-2147483648,-2147483648,-2147483648]]):
        yield case(spring(fluid),legacy=legacy,seed=-1,fail=fail,reject=reject,origin=origin,repeats=2)
    for valid in [[],['air','stone'],'#minecraft:logs','#minecraft:leaves',['stone','water','oak_log'],'air']:
        for fallback in ['stone','air','oak_log','oak_leaves']:
            yield case(spring(valid_blocks=valid,requires_block_below=False),fallback=get(fallback))
    for key,values in [('state',[None,False,'water',{}, {'id':'missing'}, {'id':'water','properties':{'falling':False}}]),
                       ('requires_block_below',[None,False,True,0,1,'true']),
                       ('rock_count',[None,-1,0,4,5,6,2147483647,-2147483648,0.5,False,'4',4294967300]),
                       ('hole_count',[None,-1,0,1,5,6,2147483647,False,'1']),
                       ('valid_blocks',[None,False,{},[],['missing'],'#missing'])]:
        for value in values:
            definition=spring();definition[key]=value;yield case(definition)
        definition=spring();definition.pop(key,None);yield case(definition)
    for legacy,seed,fail,reject in itertools.product([False,True],[0,-1,0x123456789abcdef],[-1,0,5,12,14],[0,3]):
        placed={'feature':spring(),'placement':[{'type':'count','count':2}]}
        yield case(placed,mode='placed',legacy=legacy,seed=seed,fail=fail,reject=reject,repeats=2)
        yield case({'type':'overlay','features':[placed,{'feature':'horizon:moss','placement':[]}]},legacy=legacy,seed=seed,fail=fail,reject=reject)


if __name__=='__main__':
    harness.run(corpus,'rust-spring-parity',__file__,
        'Native SpringFeature codec and ordered cavity checks. Every block state is exercised above, below, at the origin and as a horizontal neighbor; all air/rock/other five-neighbor combinations and requested count pairs, overlapping air/valid-block membership, defaults, malformed fields, tags, wrapped coordinates and all 37 fluid states are included. Separate ordered rock and hole reads, flags 2, rejected writes, immediate fluid tick requests even after rejected writes, partial failures and untouched random streams are compared. Scheduled requests, named providers and overlay/modifier composition are traced; execution of fluid ticks, spreading and full gameplay remain pending.')
