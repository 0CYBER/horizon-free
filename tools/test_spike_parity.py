#!/usr/bin/env python3
"""Compare tapered spikes, rare tall forms and discontinuous roots."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']
    get = lambda name: facts['defaults']['minecraft:' + name]

    def spike(**extra):
        value = {'type': 'spike', 'state': {'id': 'packed_ice'},
                 'can_place_on': {'type': 'matching_blocks', 'blocks': 'snow_block'},
                 'can_replace': {'type': 'matching_blocks', 'blocks': ['snow_block', 'ice']}}
        value.update(extra)
        return value

    def case(definition=None, **extra):
        origin = extra.pop('origin', [0, 70, 0])
        value = {'definition': json.dumps(definition if definition is not None else spike(), separators=(',', ':')),
                 'mode': 'feature', 'origin': origin, 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 15, 'fallback': get('air'),
                 'states': [[*origin, get('snow_block')]], 'repeats': 1}
        value.update(extra)
        return value

    for state in states:
        yield case(spike(state=state), fail=20)
    for state in range(len(states)):
        yield case(origin=[0, -62, 0], states=[[0, -62, 0, state]], fail=20)
        yield case(fallback=state, fail=30)
    # Full runs include the rare elevated branch and both random algorithms.
    for legacy, seed in itertools.product([False, True], range(1024)):
        yield case(legacy=legacy, seed=seed)
    predicates = [
        {'type': 'true'}, {'type': 'not', 'predicate': {'type': 'true'}},
        {'type': 'matching_blocks', 'blocks': '#minecraft:logs'},
        {'type': 'has_sturdy_face', 'direction': 'up'},
        {'type': 'would_survive', 'state': 'short_grass'}, {'type': 'solid'},
    ]
    for predicate, legacy, seed, fail, reject in itertools.product(
            predicates, [False, True], range(8), [-1, 0, 1, 9, 31, 200], [0, 1, 3]):
        yield case(spike(can_replace=predicate), legacy=legacy, seed=seed, fail=fail, reject=reject,
                   fallback=get('oak_log'), repeats=2)
        yield case(spike(can_place_on=predicate), legacy=legacy, seed=seed, fail=fail, reject=reject,
                   fallback=get('stone'), repeats=2)
    for name, legacy, seed in itertools.product(
            ['air', 'cave_air', 'void_air', 'snow_block', 'ice', 'packed_ice', 'blue_ice', 'stone', 'water'],
            [False, True], range(16)):
        yield case(fallback=get(name), legacy=legacy, seed=seed, repeats=2)
    # Distinguish block identity from exact state identity in the root exception.
    for state, legacy in itertools.product(
            [{'id': 'oak_log', 'properties': {'axis': 'x'}}, {'id': 'oak_log'}, {'id': 'air'}], [False, True]):
        yield case(spike(state=state, can_replace={'type': 'not', 'predicate': {'type': 'true'}}),
                   fallback=get('oak_log'), legacy=legacy)
    for y, depth, legacy in itertools.product([-64, -62, -61, 49, 50, 51, 52, 64, 150, 319], range(5), [False, True]):
        initial = [[0, y - d, 0, get('air')] for d in range(depth)]
        initial.append([0, y - depth, 0, get('snow_block')])
        yield case(origin=[0, y, 0], states=initial, legacy=legacy)
    for origin, legacy in itertools.product(
            [[2147483647, 70, 2147483647], [-2147483648, 70, -2147483648],
             [0, -2147483648, 0], [0, 2147483647, 0]], [False, True]):
        yield case(origin=origin, legacy=legacy, fail=200)
    for key, values in [
            ('state', [None, False, {}, [], 'missing', {'id': 'missing'}, {'id': 'oak_log', 'properties': {'axis': 'invalid'}}]),
            ('can_place_on', [None, False, {}, [], {'type': 'missing'}]),
            ('can_replace', [None, False, {}, [], {'type': 'missing'}])]:
        for value in values:
            definition = spike(); definition[key] = value
            yield case(definition)
        definition = spike(); del definition[key]
        yield case(definition)
    for legacy, fail, reject in itertools.product([False, True], [-1, 0, 9, 101], [0, 3]):
        placed = {'feature': spike(), 'placement': [{'type': 'count', 'count': 2}]}
        yield case(placed, mode='placed', legacy=legacy, fail=fail, reject=reject, repeats=2)
        yield case({'type': 'overlay', 'features': [placed, {'feature': 'horizon:moss', 'placement': []}]},
                   legacy=legacy, fail=fail, reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-spike-parity', __file__,
        'Native SpikeFeature required state/predicate codecs, air descent, support gating, height/radius draws, rare tall spikes, float taper and edge draws, independently tested mirrored layers, and discontinuous roots with a fixed Y=50 floor. Every configured output, origin and candidate state is compared through failure-terminated prefixes. Full runs cover 1024 seeds per RNG, replacement/support predicates, exact-state root exceptions, rejected writes, repeated mutations, query failures, wrapped positions and placement/overlay composition. Rust limits pathological descent/root work to 2^20 visits. World update execution and complete generation remain pending.')
