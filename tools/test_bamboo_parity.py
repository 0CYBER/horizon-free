#!/usr/bin/env python3
"""Compare bamboo survival, podzol spread and ordered leafy caps."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']; get = lambda name: facts['defaults']['minecraft:' + name]
    wrap = lambda n: (n + 2**31) % 2**32 - 2**31
    def bamboo(probability=1): return {'type': 'bamboo', 'probability': probability}
    def case(definition=None, **extra):
        origin = extra.pop('origin', [0, 65, 0])
        value = {'definition': json.dumps(definition if definition is not None else bamboo()),
                 'mode': 'feature', 'origin': origin, 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 15, 'height': origin[1], 'fallback': get('air'),
                 'states': [[*origin, get('air')], [origin[0], wrap(origin[1]-1), origin[2], get('dirt')]], 'repeats': 1}
        value.update(extra); return value
    for state in range(len(states)):
        yield case(states=[[0, 65, 0, state], [0, 64, 0, get('dirt')]], fail=25)
        yield case(bamboo(0), states=[[0, 65, 0, get('air')], [0, 64, 0, state]], fail=25)
        yield case(fallback=state, fail=25)
        yield case(bamboo(0), states=[[0, 64, 0, get('dirt')], [0, 68, 0, state]])
    for probability, legacy, seed in itertools.product([0, 0.01, 0.5, 1], [False, True], range(64)):
        yield case(bamboo(probability), legacy=legacy, seed=seed, repeats=2)
    for legacy, seed, fail, reject in itertools.product([False, True], range(16), [-1, 0, 1, 2, 4, 11, 25, 60, 100], [0, 1, 3]):
        yield case(legacy=legacy, seed=seed, fail=fail, reject=reject, fallback=get('dirt'),
                   states=[[0, y, 0, get('air')] for y in range(65, 84)], repeats=2)
    for stop, legacy, reject in itertools.product(range(1, 19), [False, True], [0, 1, 3]):
        yield case(bamboo(0), legacy=legacy, reject=reject, states=[[0, 64, 0, get('dirt')], [0, 65+stop, 0, get('bedrock')]])
    for height, legacy in itertools.product([-2147483648, -64, 0, 65, 320, 2147483647], [False, True]):
        yield case(height=height, legacy=legacy)
    for origin, legacy in itertools.product(
            [[2147483647, 65, 2147483647], [-2147483648, 65, -2147483648], [0, 2147483647, 0], [0, -2147483648, 0],
             [2147483644, 65, 2147483644]], [False, True]):
        yield case(origin=origin, legacy=legacy, fail=20)
        yield case(bamboo(0), origin=origin, legacy=legacy)
    for value in [None, False, -1, -0.0, -1e-100, 1e-100, 1.0000000001, 1.01, '0.5', [], {}]:
        yield case(bamboo(value))
    yield case({'type': 'bamboo'})
    for legacy, fail, reject in itertools.product([False, True], [-1, 0, 9, 31], [0, 3]):
        placed = {'feature': bamboo(), 'placement': [{'type': 'count', 'count': 2}]}
        yield case(placed, mode='placed', legacy=legacy, fail=fail, reject=reject, repeats=2)
        yield case({'type': 'overlay', 'features': [placed, {'feature': 'horizon:moss', 'placement': []}]}, legacy=legacy, fail=fail, reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-bamboo-parity', __file__,
        'Native BambooFeature bounded float codec, air-origin gate, actual bamboo survival, attempted-success semantics, height and optional radius draws, circular podzol patch with ordered WORLD_SURFACE requests, replacement tag, stalk air checks and three leafy caps including the stopping-position overwrite. All origin/support/podzol states have failure-terminated prefixes; all obstacle states have full no-podzol runs. Both RNGs, many seeds, signed zero and float rounding, obstruction heights, rejected writes, repeated mutation, partial failures, wrapped coordinates and composition are covered. Fixture height values are controlled and do not update after writes. Growth, world updates and complete generation remain pending; pathological wrapped patch loops are bounded to 2^20 visits.')
