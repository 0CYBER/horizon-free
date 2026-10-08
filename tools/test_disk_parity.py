#!/usr/bin/env python3
"""Compare disk columns, optional providers and postprocessing query order."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']; get = lambda name: facts['defaults']['minecraft:' + name]
    def disk(provider=None, **extra):
        value = {'type': 'disk', 'state_provider': provider if provider is not None else {'id': 'clay'},
                 'target': {'type': 'true'}, 'radius': 0, 'half_height': 0}
        value.update(extra); return value
    def case(definition=None, **extra):
        value = {'definition': json.dumps(definition if definition is not None else disk(), separators=(',', ':')),
                 'mode': 'feature', 'origin': [0, 64, 0], 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 15, 'fallback': get('stone'), 'states': [], 'repeats': 2}
        value.update(extra); return value
    for state in states:
        yield case(disk(state))
    for state in range(len(states)):
        yield case(states=[[0, 65, 0, state], [0, 66, 0, state]])
    empty = {'type': 'random_block', 'blocks': []}
    selective = {'type': 'rule_based', 'fallback': {'id': 'clay'}, 'rules': [
        {'if_true': {'type': 'matching_blocks', 'blocks': 'water'}, 'then': empty}]}
    providers = [{'id': 'clay'}, empty, selective, {'type': 'random_block', 'blocks': ['clay', 'air', 'water']},
                 {'type': 'rotated', 'state': {'id': 'oak_log'}}, 'horizon:stone']
    targets = [{'type': 'true'}, {'type': 'not', 'predicate': {'type': 'true'}},
               {'type': 'matching_blocks', 'blocks': ['stone', 'water']},
               {'type': 'solid'}, {'type': 'replaceable'}, {'type': 'has_sturdy_face', 'direction': 'up'}]
    for provider, target, legacy, seed, fail, reject in itertools.product(
            providers, targets, [False, True], [0, -1, 0x123456789abcdef], [-1, 0, 1, 2, 4, 7, 12], [0, 1, 3]):
        yield case(disk(provider, target=target, radius={'type': 'uniform', 'min_inclusive': 0, 'max_inclusive': 2}, half_height=2),
                   states=[[0, 65, 0, get('water')], [0, 63, 0, get('air')]], legacy=legacy, seed=seed, fail=fail, reject=reject)
    # Optional-provider gaps retain the preceding placement flag; failed target
    # tests reset it. Air above a column stops the two-position postprocessing.
    for middle, above, fail in itertools.product(['water', 'air', 'stone'], ['air', 'cave_air', 'void_air', 'stone'], [-1, *range(22)]):
        yield case(disk(selective, target={'type': 'matching_blocks', 'blocks': ['stone', 'water']}, half_height=2),
                   states=[[0, 65, 0, get(middle)], [0, 67, 0, get(above)], [0, 63, 0, get('air')]], fail=fail)
    for radius, height in itertools.product(range(9), range(5)):
        yield case(disk(radius=radius, half_height=height), repeats=1)
    for x, y, z in itertools.product([2147483647, -2147483648], [-64, 319, 2147483647, -2147483648], [2147483647, -2147483648]):
        for height in [0, 1, 4]:
            # Bottom overflow can make a native column enormous; interrupt its
            # first query after comparing the exact traversal prefix.
            yield case(disk(radius=2, half_height=height), origin=[x,y,z], fail=0 if y == -2147483648 else -1, repeats=1)
    for origin in [[2147483647,64,0], [0,64,-2147483648]]:
        yield case(disk(radius=2), origin=origin, fail=0, repeats=1)
    for key, values in [('state_provider',[None,False,{},'missing']), ('target',[None,False,{}]),
                        ('radius',[None,-1,0,8,9,{'type':'uniform','min_inclusive':0,'max_inclusive':9}]),
                        ('half_height',[None,-1,0,4,5,False,'1'])]:
        for value in values:
            definition=disk();definition[key]=value;yield case(definition)
        definition=disk();del definition[key];yield case(definition)
    for legacy, fail, reject in itertools.product([False,True],[-1,0,3,10],[0,3]):
        placed={'feature':disk(selective,radius=1,half_height=1),'placement':[{'type':'count','count':2},{'type':'in_square'}]}
        yield case(placed,mode='placed',legacy=legacy,fail=fail,reject=reject)
        yield case({'type':'overlay','features':[placed,{'feature':'horizon:moss','placement':[]}]},legacy=legacy,fail=fail,reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-disk-parity', __file__,
        'Native DiskFeature codec and descending replacement columns, circular inclusive-box traversal, every candidate state, every state above a replacement, all valid radius/half-height constants and bounded integer provider decoding. Optional-provider gaps retain prior placement while rejected targets reset it. Two-block markAboveForPostProcessing reads stop on any air variant; chunk resolution and native ProtoChunk mark calls are traced with ordered failures. Provider draws, repeated mutations, rejected writes, both random modes and overlay/modifier composition are compared. Wrapped finite boxes and failure-terminated pathological prefixes are covered; Rust bounds visits to 2^20. This fixture checks postprocessing call order, not subsequent chunk postprocessing execution. Complete chunk generation and gameplay remain pending.')
