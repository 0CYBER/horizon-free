#!/usr/bin/env python3
"""Compare vanilla random ticks (and scheduled ticks) with horizon-world, state by state.

Every block state is put at the origin of a random neighbourhood and `randomTick`
(or `tick`) is run with a seeded RNG on both sides; the resulting block changes,
scheduled ticks, side effects and the RNG fingerprint after the call must match.
"""
import random
try:
    from . import block_update_harness as h
except ImportError:
    import block_update_harness as h


def cases(facts, count, seed, only):
    rng = random.Random(seed)
    pool = h.state_pool(facts, only)
    for n in range(count):
        state = pool[n % len(pool)]
        case = {**h.environment(rng), 'pos': [0, 0, 0], 'cells': facts.cells(rng, state),
                'mode': 'random' if n % 4 else 'tick'}
        yield case


def main():
    args = h.argument_parser(__doc__).parse_args()
    facts = h.load_facts()
    differing = h.execute('random-tick-parity', list(cases(facts, args.count, args.seed, args.only)), args, facts)
    h.finish(differing)


if __name__ == '__main__':
    main()
