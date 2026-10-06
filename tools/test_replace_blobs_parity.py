#!/usr/bin/env python3
"""Compare replacement-blob descent, three radius draws and Manhattan order."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']; get = lambda name: facts['defaults']['minecraft:' + name]
    def blob(**extra):
        value = {'type': 'netherrack_replace_blobs', 'target': {'id': 'netherrack'}, 'state': {'id': 'basalt'}, 'radius': 2}
        value.update(extra); return value
    def case(definition=None, **extra):
        value = {'definition': json.dumps(definition if definition is not None else blob(), separators=(',', ':')),
                 'mode': 'feature', 'origin': [0, 64, 0], 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 15, 'fallback': get('netherrack'), 'states': [], 'repeats': 1}
        value.update(extra); return value
    for state_id, state in enumerate(states):
        yield case(blob(target=state, radius=0), fallback=state_id)
        yield case(blob(state=state, radius=0))
        yield case(fallback=state_id, states=[[0, 64, 0, get('netherrack')]], fail=30)
    for radius, legacy, seed in itertools.product(range(13), [False, True], range(8)):
        yield case(blob(radius=radius), legacy=legacy, seed=seed)
    providers = [
        {'type': 'uniform', 'min_inclusive': 0, 'max_inclusive': 12},
        {'type': 'biased_to_bottom', 'min_inclusive': 0, 'max_inclusive': 12},
        {'type': 'very_biased_to_bottom', 'min_inclusive': 0, 'max_inclusive': 12},
        {'type': 'trapezoid', 'min': 0, 'max': 12, 'plateau': 4},
        {'type': 'clamped', 'source': {'type': 'uniform', 'min_inclusive': -12, 'max_inclusive': 20}, 'min_inclusive': 0, 'max_inclusive': 12},
        {'type': 'weighted_list', 'distribution': [{'weight': 1, 'data': 0}, {'weight': 3, 'data': 6}, {'weight': 1, 'data': 12}]},
    ]
    for provider, legacy, seed, fail, reject in itertools.product(providers, [False, True], range(8), [-1, 0, 1, 7, 29], [0, 1, 3]):
        yield case(blob(radius=provider), legacy=legacy, seed=seed, fail=fail, reject=reject, repeats=2)
    for y, depth in itertools.product([-2147483648, -65, -64, -63, -62, -61, 0, 319, 320, 2147483647], range(6)):
        start = min(319, max(-63, y))
        initial = [[0, start-d, 0, get('air')] for d in range(depth)]
        yield case(origin=[0, y, 0], states=initial)
        yield case(origin=[0, y, 0], fallback=get('air'))
    for target in [{'id': 'oak_log', 'properties': {'axis': 'x'}}, {'id': 'oak_log'}, {'id': 'air'}]:
        yield case(blob(target=target), fallback=get('oak_log'))
    for origin, legacy in itertools.product([[2147483647, 64, -2147483648], [-2147483648, 64, 2147483647]], [False, True]):
        yield case(blob(radius=12), origin=origin, legacy=legacy)
    for key, values in [
            ('target', [None, False, {}, [], 'missing', {'id': 'missing'}]),
            ('state', [None, False, {}, [], 'missing', {'id': 'oak_log', 'properties': {'axis': 'invalid'}}]),
            ('radius', [None, False, -1, 13, -0.0, 0.9, 12.9, '2', {}, [], {'type': 'uniform', 'min_inclusive': -1, 'max_inclusive': 2}, {'type': 'constant', 'value': 13}])]:
        for value in values:
            definition = blob(); definition[key] = value
            yield case(definition)
        definition = blob(); del definition[key]
        yield case(definition)
    for legacy, fail, reject in itertools.product([False, True], [-1, 0, 9, 101], [0, 3]):
        placed = {'feature': blob(), 'placement': [{'type': 'count', 'count': 2}]}
        yield case(placed, mode='placed', legacy=legacy, fail=fail, reject=reject, repeats=2)
        yield case({'type': 'overlay', 'features': [placed, {'feature': 'horizon:moss', 'placement': []}]}, legacy=legacy, fail=fail, reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-replace-blobs-parity', __file__,
        'Native ReplaceBlobsFeature under its netherrack_replace_blobs registry key: target block identity, required replacement state and radius provider bounded 0..12, build-height clamp, descending target search, three radius draws, clipped Manhattan shells in native depth/X/Y/positive-Z/mirrored-Z order, flags 3 and success after rejected writes. Every configured target and output state has a radius-zero run; all candidate states have failure-terminated prefixes. Full radius/provider runs cover both RNGs, boundaries, wrapped X/Z, repeated mutations, query failures and composition. Pathological descent is bounded to 2^20 visits. World updates and full generation remain pending.')
