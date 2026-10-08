#!/usr/bin/env python3
"""Compare native biome freezing and snow eligibility over real block states."""
import itertools
try:
    from . import test_simple_block_parity as harness
except ImportError:
    import test_simple_block_parity as harness


def corpus(facts, placement):
    states = facts['states']; get = lambda name: facts['defaults']['minecraft:' + name]
    directions = [(-1, 0, 0), (1, 0, 0), (0, 0, -1), (0, 0, 1)]
    def case(operation='freeze', **extra):
        origin = extra.pop('origin', [0, 64, 0])
        value = {'definition': '{}', 'mode': 'weather', 'operation': operation,
                 'base': '0', 'modifier': 'NONE', 'precipitation': True, 'edge': False,
                 'origin': origin, 'legacy': False, 'seed': 0, 'world_seed': 1,
                 'fail': -1, 'reject': 0, 'brightness': 0, 'sea': 63,
                 'fallback': get('stone'), 'states': [[*origin, get('water' if operation == 'freeze' else 'air')]], 'repeats': 1}
        value.update(extra); return value
    for state in range(len(states)):
        yield case(states=[[0, 64, 0, state]])
        yield case(states=[[0, 64, 0, state]], fluid=get('water'))
        yield case(fluid=state)
        yield case('snow', states=[[0, 64, 0, state]])
        yield case('snow', states=[[0, 64, 0, get('air')], [0, 63, 0, state]])
        for x, y, z in directions:
            yield case(edge=True, fallback=get('water'), states=[[x, 64+y, z, state]])
    for mask, legacy, fail in itertools.product(range(16), [False, True], [-1, 0, 1, 2, 3, 4, 5, 6]):
        yield case(edge=True, legacy=legacy, fail=fail,
                   states=[[0, 64, 0, get('water')], *[[x, 64+y, z, get('water') if mask & (1 << i) else get('stone')] for i, (x, y, z) in enumerate(directions)]])
    for operation, base, modifier, precipitation, y, light in itertools.product(
            ['freeze', 'snow'], ['0', '80000000', '3e199999', '3e19999a', '3e19999b', '3e4ccccd', '3f800000', 'bf800000', '7f800000', 'ff800000', '7fc00000', 'ffc00001'],
            ['NONE', 'FROZEN'], [False, True], [-65, -64, 0, 64, 80, 81, 319, 320], [-1, 0, 9, 10, 15]):
        yield case(operation, base=base, modifier=modifier, precipitation=precipitation, origin=[12345, y, -9876], brightness=light, repeats=2)
    for operation, legacy, sea, fail in itertools.product(['freeze', 'snow'], [False, True], [-2147483648, -1000, 0, 63, 2147483647], [-1, 0, 1, 2, 3, 4, 5, 6]):
        yield case(operation, origin=[2147483647, 64, -2147483648], legacy=legacy, sea=sea, fail=fail, edge=True, repeats=2)
    for operation, origin in itertools.product(['freeze', 'snow'], [[0, -2147483648, 0], [0, 2147483647, 0], [-2147483648, -64, 2147483647]]):
        yield case(operation, origin=origin, edge=True)
    for name, level, legacy in itertools.product(['air', 'cave_air', 'void_air', 'snow', 'water', 'oak_slab', 'honey_block', 'ice', 'packed_ice'], range(16), [False, True]):
        yield case('snow', states=[[0, 64, 0, get(name)], [0, 63, 0, get('stone')]], brightness=level, legacy=legacy)


if __name__ == '__main__':
    harness.run(corpus, 'rust-biome-weather-parity', __file__,
        'Actual native Biome.shouldFreeze and Biome.shouldSnow using private climate construction for explicit float bits, native temperature modifiers/cache, real fluid identities, real snow survival/support and light/build-height gates. All states are compared as freezing blocks, forced fluid carriers, contained fluids, snow candidates, snow supports and each water-edge neighbor. All neighbor masks, both RNG tails, repeated queries, raw float thresholds/nonfinite values, both temperature modifiers, precipitation flags, light boundaries, sea-level extremes, partial failures and wrapped coordinates are included. Dynamic block entities are absent; lighting propagation, mutation, snow/ice updates and complete weather simulation remain pending. Native per-biome thread-local caches are released only after each independent fixture, retaining repeated-query semantics within it. The fixture mode directly calls biome methods and does not represent a vanilla feature codec.')
