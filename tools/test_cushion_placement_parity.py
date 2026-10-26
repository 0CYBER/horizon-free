#!/usr/bin/env python3
"""Compare cushion support, burial and coverage against vanilla's real checks."""
import json
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def make_input(build):
    full = [[0, 0, 0, 1, 1, 1]]
    shapes = [[], full, [[0, 0, 0, 1, .5, 1]], [[0, .5, 0, 1, 1, 1]],
              [[.375, 0, .375, .625, 1.5, .625]], [[0, 0, 0, .5, 1, 1]],
              [[0, 0, 0, .5, 1, 1], [.5, 0, 0, 1, 1, 1]]]
    rows = []
    for x, z in ((0, 0), (-2, -3), (17, 31)):
        for y in (0, .125, .5, .875, 1, 1.00000001, 1.015625, 1.125):
            for outline in shapes:
                for collision in shapes:
                    for suffocating in (False, True):
                        rows.append(dict(pos=[x + .5, y, z + .5],
                                         cells=[[x, 0, z, outline, collision, suffocating]]))
    rng = random.Random(1935)
    for _ in range(500):
        cells = []
        for y in (-1, 0, 1):
            for x in (-1, 0, 1):
                for z in (-1, 0, 1):
                    if rng.random() < .3:
                        cells.append([x, y, z, rng.choice(shapes), rng.choice(shapes), bool(rng.randrange(2))])
        rows.append(dict(pos=[rng.choice((0, .5, 1)), rng.choice((0, .5, 1, 1.125)), rng.choice((0, .5, 1))], cells=cells))
    path = build / 'inputs.jsonl'
    path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    return path, len(rows)


def main():
    harness.differential(
        'rust-cushion-placement-parity', 'VanillaCushionPlacementOracle',
        'cushion_placement_probe', make_input,
        'Actual Cushion.canBePlacedAt and wouldSurviveAt, with controlled outline, collision '
        'and suffocation queries. Includes partial supports, burial, complete coverage, '
        'overhanging outlines, negative coordinates and boundary heights. Item use, '
        'entity tracking, interaction and persistence are checked separately.',
        sources=[lab.ROOT / 'crates/horizon-client/src/cushion.rs'])


if __name__ == '__main__':
    main()
