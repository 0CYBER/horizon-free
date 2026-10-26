#!/usr/bin/env python3
"""Compare the real Squid.aiStep client branch and cycle event with Rust."""
import json
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def make_input(build):
    rng = random.Random(1684)
    rows = []
    for species in ('squid', 'glow_squid'):
        for water in (False, True):
            for reset in (False, True):
                for velocity in ((0, 0, 0), (.1, 0, 0), (0, .1, 0),
                                 (0, -.1, 0), (0, 0, -.1), (.1, .2, -.3)):
                    for movement in (0, 1, 2.4, 3.14, 4, 6.2, 6.4):
                        rows.append(dict(species=species, water=water, reset=reset,
                                         x=velocity[0], y=velocity[1], z=velocity[2],
                                         body=rng.uniform(-180, 180),
                                         v=[movement, .2, -45, 33, .15, .8]))
    for _ in range(1000):
        rows.append(dict(species=rng.choice(('squid', 'glow_squid')), water=True,
                         reset=False, x=rng.uniform(.01, .2), y=rng.uniform(.01, .2),
                         z=rng.uniform(.01, .2), body=rng.uniform(-180, 180),
                         v=[rng.uniform(0, 7), 0, rng.uniform(-90, 90),
                            rng.uniform(0, 180), rng.uniform(.1, .2), rng.random()]))
    path = build / 'inputs.jsonl'
    path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    return path, len(rows)


def main():
    harness.differential(
        'rust-squid-client-parity', 'VanillaSquidClientOracle', 'squid_client_probe',
        make_input,
        'Actual Squid.aiStep on a client with controlled inherited movement/world services. '
        'Checks water/air body rotation and tentacle state, cycle clamping and event 19. '
        'Glow squid uses the same inherited method; packet dispatch and rendering are separate.',
        sources=[lab.ROOT / 'crates/horizon-client/src/mobs/aquatic.rs'])


if __name__ == '__main__':
    main()
