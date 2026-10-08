#!/usr/bin/env python3
"""Compare delta cavity checks, rim placement and mutable shifted contents."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']; get = lambda name: facts['defaults']['minecraft:' + name]
    directions = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)]
    def delta(**extra):
        value = {'type': 'delta_feature', 'contents': {'id': 'lava'}, 'rim': {'id': 'magma_block'}, 'size': 0, 'rim_size': 0}
        value.update(extra); return value
    def case(definition=None, **extra):
        value = {'definition': json.dumps(definition if definition is not None else delta(), separators=(',', ':')),
                 'mode': 'feature', 'origin': [0, 64, 0], 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 15, 'fallback': get('netherrack'), 'air_above': 64, 'states': [], 'repeats': 1}
        value.update(extra); return value
    for state_id, state in enumerate(states):
        yield case(states=[[0, 64, 0, state_id]])
        yield case(delta(contents=state))
        yield case(delta(rim=state, rim_size=1))
        for x, y, z in directions:
            yield case(states=[[x, 64+y, z, state_id]])
    for mask, legacy, reject in itertools.product(range(64), [False, True], [0, 1, 3]):
        yield case(delta(size=2, rim_size=1), legacy=legacy, reject=reject,
                   states=[[x, 64+y, z, get('air') if mask & (1 << i) else get('stone')] for i, (x, y, z) in enumerate(directions)], repeats=2)
    for size, rim, legacy, seed in itertools.product(range(17), [0, 1, 4, 16], [False, True], [0, -1]):
        yield case(delta(size=size, rim_size=rim), legacy=legacy, seed=seed)
    providers = [
        {'type': 'uniform', 'min_inclusive': 0, 'max_inclusive': 4},
        {'type': 'biased_to_bottom', 'min_inclusive': 0, 'max_inclusive': 4},
        {'type': 'very_biased_to_bottom', 'min_inclusive': 0, 'max_inclusive': 4},
        {'type': 'trapezoid', 'min': 0, 'max': 4, 'plateau': 2},
        {'type': 'clamped', 'source': {'type': 'uniform', 'min_inclusive': -4, 'max_inclusive': 8}, 'min_inclusive': 0, 'max_inclusive': 4},
        {'type': 'weighted_list', 'distribution': [{'weight': 1, 'data': 0}, {'weight': 2, 'data': 4}]},
    ]
    for provider, legacy, seed, fail, reject in itertools.product(providers, [False, True], range(8), [-1, 0, 1, 7, 15, 31], [0, 1, 3]):
        yield case(delta(size=provider, rim_size=provider), legacy=legacy, seed=seed, fail=fail, reject=reject, repeats=2)
    for contents, rim in itertools.product(['air', 'cave_air', 'netherrack', 'magma_block', 'lava', 'bedrock', 'chest'], repeat=2):
        yield case(delta(contents={'id': contents}, rim={'id': rim}, size=4, rim_size=2), repeats=2)
    for legacy, seed in itertools.product([False, True], range(128)):
        yield case(delta(size=2, rim_size={'type': 'uniform', 'min_inclusive': 0, 'max_inclusive': 2}), legacy=legacy, seed=seed)
    for origin, legacy in itertools.product([[2147483647, 64, -2147483648], [-2147483648, 64, 2147483647], [0, -2147483648, 0], [0, 2147483647, 0]], [False, True]):
        yield case(delta(size=4, rim_size=4), origin=origin, air_above=origin[1], legacy=legacy)
    for key, values in [
            ('contents', [None, False, {}, [], 'missing', {'id': 'missing'}]),
            ('rim', [None, False, {}, [], 'missing', {'id': 'oak_log', 'properties': {'axis': 'invalid'}}]),
            ('size', [None, False, -1, 17, 16.9, '2', {}, [], {'type': 'uniform', 'min_inclusive': -1, 'max_inclusive': 2}]),
            ('rim_size', [None, False, -1, 17, 16.9, '2', {}, []])]:
        for value in values:
            definition = delta(); definition[key] = value
            yield case(definition)
        definition = delta(); del definition[key]
        yield case(definition)
    for legacy, fail, reject in itertools.product([False, True], [-1, 0, 9, 101], [0, 3]):
        placed = {'feature': delta(size=2, rim_size=1), 'placement': [{'type': 'count', 'count': 2}]}
        yield case(placed, mode='placed', legacy=legacy, fail=fail, reject=reject, repeats=2)
        yield case({'type': 'overlay', 'features': [placed, {'feature': 'horizon:moss', 'placement': []}]}, legacy=legacy, fail=fail, reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-delta-parity', __file__,
        'Native DeltaFeature required state and bounded size-provider codecs, 0.9 double rim draw, conditional rim X/Z sampling, independent patch X/Z sampling, Manhattan traversal and immediate rim/shifted-content mutations. Every state is tested as origin, contents, rim and all six neighbors; all air-neighbor masks, size/rim limits, six provider families, zero-component rim offsets, both RNGs, seeds, repeated mutations, rejected writes, partial failures, wrapped coordinates and composition are covered. The world fixture has a controlled flat air/ground boundary with mutable overrides. World updates, fluid behavior and full generation remain pending.')
