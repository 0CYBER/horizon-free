#!/usr/bin/env python3
"""Compare top-layer ice, snow, biome queries and snowy ground mutations."""
import itertools
import json
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']; get = lambda name: facts['defaults']['minecraft:' + name]
    def case(**extra):
        value = {'definition': '{"type":"freeze_top_layer"}', 'mode': 'feature',
                 'biome': {'base': '0', 'modifier': 'NONE', 'precipitation': True},
                 'origin': [0, 100, 0], 'height': 65, 'air_above': 64, 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 0, 'sea': 63, 'fallback': get('grass_block'), 'states': [], 'repeats': 1}
        value.update(extra); return value
    for state in range(len(states)):
        yield case(fallback=state, fail=24)
        yield case(states=[[0, 65, 0, state]], fail=24)
        yield case(fallback=get('water'), fluid=state, fail=24)
    for base, modifier, precipitation, light, name, legacy in itertools.product(
            ['0', '3e199999', '3e19999a', '3e4ccccd', '7fc00000'], ['NONE', 'FROZEN'], [False, True], [0, 9, 10],
            ['grass_block', 'podzol', 'mycelium', 'water', 'ice', 'stone', 'air', 'snow'], [False, True]):
        yield case(biome={'base': base, 'modifier': modifier, 'precipitation': precipitation}, brightness=light, fallback=get(name), legacy=legacy, repeats=2)
    for name, fail, reject in itertools.product(['water', 'grass_block', 'podzol', 'mycelium', 'stone'], [-1, 0, 1, 2, 3, 5, 9, 17, 31, 100], [0, 1, 3]):
        yield case(fallback=get(name), fail=fail, reject=reject, repeats=2)
    for y, sea in itertools.product([-2147483648, -64, -63, 0, 64, 80, 81, 319, 320, 2147483647], [-2147483648, 0, 63, 2147483647]):
        yield case(height=y, air_above=y-1 if y>-2147483648 else y, sea=sea, repeats=2)
    for origin in [[-17, -2147483648, -33], [2147483647, 2147483647, -2147483648], [-2147483648, 0, 2147483647]]:
        yield case(origin=origin, repeats=2)
    yield case(definition='{"type":"freeze_top_layer","ignored":null}')
    for fail, reject in itertools.product([-1, 0, 9, 101], [0, 3]):
        placed = {'feature': {'type': 'freeze_top_layer'}, 'placement': [{'type': 'count', 'count': 2}]}
        yield case(definition=json.dumps(placed), mode='placed', fail=fail, reject=reject)
        yield case(definition=json.dumps({'type': 'overlay', 'features': [placed, {'feature': 'horizon:moss', 'placement': []}]}), fail=fail, reject=reject)


if __name__ == '__main__':
    harness.run(corpus, 'rust-snow-freeze-parity', __file__,
        'Native SnowAndFreezeFeature unit codec, 16x16 X-then-Z traversal, MOTION_BLOCKING heights, one biome lookup per column retained across freeze/snow checks, actual biome temperature/freezing/snow predicates, ice-before-snow mutation, snowy ground state changes after attempted snow writes, flags 2 and unconditional normal success. Every ground, snow candidate and fluid-state carrier is compared through failure-terminated prefixes. Full runs cover climate thresholds/NaN, both modifiers, precipitation, light gates, both unchanged RNGs, repeated mutations/cache use, rejected writes, partial failures, extreme heights/sea levels, wrapped X/Z and composition. Fixture heights and a single biome are controlled; actual lighting, dynamic biome selection, world update execution and full generation remain pending. Native temperature caches are released after each independent fixture.')
