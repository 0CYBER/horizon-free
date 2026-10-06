#!/usr/bin/env python3
"""Compare native recursive chorus generation and connection-state refreshes."""
import itertools
import json
import random
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    get=lambda name:facts['defaults']['minecraft:'+name]
    ground=[0,63,0,get('end_stone')]
    def case(**extra):
        value={'definition':json.dumps({'type':'chorus_plant'}),'mode':'feature','origin':[0,64,0],
               'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,'brightness':15,
               'fallback':get('air'),'states':[ground],'repeats':1}
        value.update(extra);return value
    for state in range(len(facts['states'])):
        yield case(states=[[0,64,0,state],ground],fail=2)
        yield case(states=[[0,63,0,state]],fail=2)
        # Full initial connection-state evaluation before the injected write failure.
        for x,y,z in [[0,65,0],[0,64,-1],[1,64,0],[0,64,1],[-1,64,0]]:
            yield case(states=[ground,[x,y,z,state]],fail=8)
    for legacy,seed in itertools.product([False,True],range(1024)):
        yield case(legacy=legacy,seed=seed,repeats=2)
    for legacy,seed,fail,reject in itertools.product([False,True],[0,1,-1],[-1,*range(80),101,127,191,255],[0,1,3]):
        yield case(legacy=legacy,seed=seed,fail=fail,reject=reject,repeats=2)
    rng=random.Random(0x43484f525553)
    palette=[get(n) for n in ['air','cave_air','void_air','stone','end_stone','chorus_plant','chorus_flower','water']]
    for legacy in [False,True]:
        for _ in range(256):
            entries=[ground]+[[rng.randrange(-7,8),rng.randrange(65,84),rng.randrange(-7,8),rng.choice(palette)] for _ in range(80)]
            yield case(legacy=legacy,seed=rng.randrange(2**63),states=entries,reject=rng.choice([0,1,3]),fail=rng.choice([-1,19,37,100,300]))
    for legacy,origin,seed in itertools.product([False,True],[[0,-64,0],[-17,319,-33],[2147483647,2147483647,-2147483648],[-2147483648,-2147483648,2147483647]],[0,1,-1]):
        below=[origin[0],((origin[1]-1+2**31)%2**32)-2**31,origin[2],get('end_stone')]
        yield case(legacy=legacy,origin=origin,seed=seed,states=[below])
    for legacy,seed,fail,reject in itertools.product([False,True],[0,-1],[-1,0,8,20,100],[0,1,3]):
        placed={'feature':{'type':'chorus_plant'},'placement':[{'type':'count','count':2}]}
        yield case(definition=json.dumps(placed),mode='placed',legacy=legacy,seed=seed,fail=fail,reject=reject)
        yield case(definition=json.dumps({'type':'overlay','features':[placed,{'feature':{'type':'chorus_plant'},'placement':[]}]}),legacy=legacy,seed=seed,fail=fail,reject=reject)
    yield case(definition=json.dumps({'type':'chorus_plant','ignored':False}))


if __name__=='__main__':
    harness.run(corpus,'rust-chorus-plant-parity',__file__,
        'Native ChorusPlantFeature unit codec, origin air/support gates, ChorusFlowerBlock.generatePlant and depth-bounded recursive growth, horizontal clearance with parent exclusion, branch-distance bounds, connection refreshes in below/above/north/east/south/west read order and age-five terminal flowers. Every state exercises root/support gates and five neighbor connection prefixes. Full runs cover 1024 seeds per RNG, every early world-call failure, rejected writes, repeated mutation, random obstacles, wrapped coordinates and feature composition. Ordered reads/writes, final state maps and subsequent RNG draws are compared. Neighbor-update execution, random tick growth, drops, rendering and complete End generation remain pending.')
