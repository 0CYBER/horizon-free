#!/usr/bin/env python3
"""Compare native bonus chest candidate search, loot assignment and torches."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    get=lambda name:facts['defaults']['minecraft:'+name]
    def case(**extra):
        value={'definition':json.dumps({'type':'bonus_chest'}),'mode':'feature','origin':[0,64,0],
               'legacy':False,'seed':0,'world_seed':1,'fail':-1,'reject':0,'brightness':15,
               'fallback':get('air'),'states':[],'repeats':1,'loot_entity':'chest'}
        value.update(extra);return value
    # Read each state's actual empty-context collision through a candidate prefix.
    for state in range(len(facts['states'])):
        yield case(fallback=state,fail=8)
    names=['air','cave_air','void_air','stone','water','lava','tall_grass','oak_leaves','oak_slab','snow','shulker_box','moving_piston','powder_snow','chest','torch']
    for legacy,seed,name,entity,fail,reject in itertools.product([False,True],[0,-1],names,['none','other','chest'],[-1,0,1,2,3,4,5,8,12,20],[0,1,3]):
        yield case(legacy=legacy,seed=seed,fallback=get(name),loot_entity=entity,fail=fail,reject=reject,repeats=2)
    for legacy,seed,entity in itertools.product([False,True],range(256),['none','chest']):
        yield case(legacy=legacy,seed=seed,loot_entity=entity,air_above=63,fallback=get('stone'),repeats=2)
    for legacy,origin,height in itertools.product([False,True],[[0,0,0],[-1,-64,-1],[16,319,16],[2147483647,-2147483648,-2147483648]],[-2147483648,-64,0,319,2147483647]):
        yield case(legacy=legacy,origin=origin,height=height,air_above=height-1 if height>-2147483648 else -2147483648,fallback=get('stone'))
    # Force search through many solid candidates before finding the sole air cell.
    for legacy,seed in itertools.product([False,True],range(32)):
        yield case(legacy=legacy,seed=seed,fallback=get('stone'),states=[[15,64,15,get('air')]])
    for legacy,seed,fail,reject in itertools.product([False,True],[0,-1,987654321],[-1,0,5,20],[0,1,3]):
        placed={'feature':{'type':'bonus_chest'},'placement':[{'type':'count','count':2},{'type':'in_square'}]}
        yield case(definition=json.dumps(placed),mode='placed',legacy=legacy,seed=seed,fail=fail,reject=reject,air_above=63,fallback=get('stone'))
        yield case(definition=json.dumps({'type':'overlay','features':[placed,{'feature':{'type':'bonus_chest'},'placement':[]}]}),legacy=legacy,seed=seed,fail=fail,reject=reject)
    yield case(definition=json.dumps({'type':'bonus_chest','ignored':None}))


if __name__=='__main__':
    harness.run(corpus,'rust-bonus-chest-parity',__file__,
        'Native BonusChestFeature unit codec, independently shuffled chunk X/Z coordinates, ordered heightmap candidate search, air short-circuit and second-read empty-context collision, chest writes, conditional randomizable-container loot seeding, north/east/south/west torch survival and writes. Every state participates in collision prefixes; complete searches cover absent/unrelated/actual native chest entities, repeated mutation, rejected writes, injected failures, both random modes, shuffled search exhaustion, sole viable columns, extreme coordinates/heights and placement composition. Actual ChestBlockEntity loot table and seed are compared; fixtures control heights and entity existence independently of writes. Loot generation, inventory behavior, automatic block-entity lifecycle, world updates and complete spawn generation remain pending.')
