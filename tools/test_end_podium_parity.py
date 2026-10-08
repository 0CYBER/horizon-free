#!/usr/bin/env python3
"""Compare End podium geometry, active replacements and ordered world effects."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']; get = lambda name: facts['defaults']['minecraft:' + name]
    def case(definition=None, **extra):
        value = {'definition': json.dumps(definition if definition is not None else {'type': 'end_podium', 'active': True}),
                 'mode': 'feature', 'origin': [0, 64, 0], 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 15, 'fallback': get('stone'), 'states': [], 'repeats': 1}
        value.update(extra); return value
    # Every state's block identity reaches native active replacement checks.
    for state in range(len(states)):
        yield case(fallback=state, fail=25)
    for active, name, legacy, fail, reject, destroy in itertools.product(
            [False, True], ['air', 'cave_air', 'void_air', 'stone', 'bedrock', 'end_stone', 'end_portal', 'wall_torch', 'water', 'chest'],
            [False, True], [-1, 0, 1, 2, 5, 19, 47, 89, 137, 200], [0, 1, 3], [False, True]):
        yield case({'type': 'end_podium', 'active': active}, fallback=get(name), legacy=legacy, fail=fail, reject=reject,
                   destroy_success=destroy, repeats=2)
    for active, origin in itertools.product([False, True],
            [[0, -64, 0], [-17, 319, -33], [2147483600, 100, -2147483600], [-2147483600, -2147483600, 2147483600]]):
        yield case({'type': 'end_podium', 'active': active}, origin=origin, repeats=2)
    for value in [None, False, True, 0, 1, -1, 0.5, 'true', 'false', [], {}]:
        yield case({'type': 'end_podium', 'active': value})
    yield case({'type': 'end_podium'})
    for legacy, fail, reject, destroy in itertools.product([False, True], [-1, 0, 9, 101], [0, 3], [False, True]):
        placed = {'feature': {'type': 'end_podium', 'active': True}, 'placement': [{'type': 'count', 'count': 2}]}
        yield case(placed, mode='placed', legacy=legacy, fail=fail, reject=reject, destroy_success=destroy)
        yield case({'type': 'overlay', 'features': [placed, {'feature': {'type': 'end_podium'}, 'placement': []}]},
                   legacy=legacy, fail=fail, reject=reject, destroy_success=destroy)


if __name__ == '__main__':
    harness.run(corpus, 'rust-end-podium-parity', __file__,
        'Native EndPodiumFeature optional active codec, inclusive box order and distance cutoffs, bedrock/end-stone/air/portal selection, central pillar and north/east/south/west wall torches. All states pass active replacement prefixes; complete representative runs cover both modes, repeated mutation, exact block identity, partial failures, rejected writes and composition. Destruction is a controlled world request with independently selected success and synthetic air replacement; drops, block entities, updates and dragon/portal gameplay are not simulated. Rust bounds pathological overflow-box work to 2^20 visits; nonoverflow extreme coordinates are tested.')
