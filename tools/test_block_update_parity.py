#!/usr/bin/env python3
"""Compare vanilla updateShape and neighborChanged with horizon-world, state by state.

Every block state is put at the origin of a random neighbourhood; `shape` cases
call `BlockState.updateShape` for one direction with a (consistent) neighbour
state and compare the resulting state, requested scheduled ticks, side effects and
RNG use; `neighbor` cases call `handleNeighborChanged` from one direction.
"""
import random
try:
    from . import block_update_harness as h
except ImportError:
    import block_update_harness as h


def cases(facts, count, seed, only):
    rng = random.Random(seed)
    pool = h.state_pool(facts, only)
    air = facts.defaults['minecraft:air']
    for n in range(count):
        state = pool[n % len(pool)]
        direction = rng.randrange(6)
        cells = facts.cells(rng, state)
        case = {**h.environment(rng), 'pos': [0, 0, 0], 'dir': direction}
        dx, dy, dz = h.DIRS[direction]
        if n % 3 == 2:
            case['mode'] = 'neighbor'
            case['from'] = [dx, dy, dz]
        else:
            case['mode'] = 'shape'
            neighbour = facts.neighbour(rng, state)
            neighbour = air if neighbour is None else neighbour
            h.put(cells, (dx, dy, dz), neighbour)
            case['neighbor'] = neighbour
        case['cells'] = cells
        yield case


def main():
    args = h.argument_parser(__doc__).parse_args()
    facts = h.load_facts()
    differing = h.execute('block-update-parity', list(cases(facts, args.count, args.seed, args.only)), args, facts)
    h.finish(differing)


if __name__ == '__main__':
    main()
