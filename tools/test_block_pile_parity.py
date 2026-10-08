#!/usr/bin/env python3
"""Compare block-pile support, ordered mutations and random consumption."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']
    get = lambda name: facts['defaults']['minecraft:' + name]
    def pile(provider=None):
        return {'type': 'block_pile', 'state_provider': provider if provider is not None else {'id': 'stone'}}
    def case(definition=None, support=None, **extra):
        origin = extra.pop('origin', [0, 64, 0])
        wrap = lambda value: (value + 2**31) % 2**32 - 2**31
        floor = [[wrap(origin[0]+x), wrap(origin[1]-1), wrap(origin[2]+z), support if support is not None else get('stone')]
                 for z in range(-3, 4) for x in range(-3, 4)]
        value = {'definition': json.dumps(definition if definition is not None else pile(), separators=(',', ':')),
                 'mode': 'feature', 'origin': origin, 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 15, 'fallback': get('air'), 'states': floor, 'repeats': 1}
        value.update(extra)
        return value
    # Every output candidate and every supporting state in the pinned registry.
    for state in states:
        yield case(pile(state))
    for state in range(len(states)):
        yield case(support=state)
    providers = [{'id': 'stone'}, {'type': 'random_block', 'blocks': []},
                 {'type': 'random_block', 'blocks': ['stone', 'water', 'hay_block']},
                 {'type': 'rotated', 'state': {'id': 'oak_log'}},
                 {'type': 'weighted', 'entries': [{'data': 'stone', 'weight': 1}, {'data': 'hay_block', 'weight': 3}]},
                 'horizon:stone']
    for provider, ground, legacy, seed, fail, reject in itertools.product(
            providers, ['stone', 'dirt_path', 'air', 'oak_slab', 'shulker_box', 'moving_piston'],
            [False, True], [0, -1, 0x123456789abcdef], [-1, 0, 1, 2, 7, 19], [0, 1, 3]):
        yield case(pile(provider), support=get(ground), legacy=legacy, seed=seed, fail=fail, reject=reject, repeats=2)
    for fallback, y, seed in itertools.product(['air', 'cave_air', 'void_air', 'water', 'short_grass', 'snow'],
                                              [-64, -60, -59, -58, 319, 320, 2147483647, -2147483648], range(4)):
        yield case(fallback=get(fallback), origin=[-17, y, -33], seed=seed)
    for x, z, seed in itertools.product([2147483647, -2147483648], [2147483647, -2147483648], range(8)):
        yield case(origin=[x, 64, z], seed=seed)
    # One overflowing horizontal dimension produces a negative native iterator
    # count; injected failures terminate after a finite matching prefix.
    for x, z in [(2147483647, 0), (0, -2147483648)]:
        for fail in [0, 1, 3]:
            yield case(origin=[x, 64, z], fail=fail)
    for value in [None, False, {}, [], 'missing', {'id': 'missing'}]:
        yield case({'type': 'block_pile', 'state_provider': value})
    yield case({'type': 'block_pile'})
    for legacy, seed, fail, reject in itertools.product([False, True], [0, -1, 0x123456789abcdef], [-1, 0, 5, 21], [0, 3]):
        placed = {'feature': pile(providers[2]), 'placement': [{'type': 'count', 'count': 2}, {'type': 'in_square'}]}
        yield case(placed, mode='placed', legacy=legacy, seed=seed, fail=fail, reject=reject)
        yield case({'type': 'overlay', 'features': [placed, {'feature': 'horizon:moss', 'placement': []}]},
                   legacy=legacy, seed=seed, fail=fail, reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-block-pile-parity', __file__,
        'Native BlockPileFeature codec and placement with every candidate and supporting block state. Minimum-height early return, radius sampling, native inclusive-box visit order, squared-distance float arithmetic, sparse outer draws, air variants, dirt-path random acceptance, cached/dynamic FULL upper support, provider selection and fallback reads, flags 260, rejected writes, partial failures, repeated calls and both random streams are compared. Named providers and overlay/modifier composition use the same world. Wrapped boxes with finite native counts and failure-terminated prefixes of negative-count boxes are included; Rust bounds pathological iteration to 2^20 visits. Block entities are absent in this fixture. Resident chunks, block updates and complete gameplay remain pending.')
