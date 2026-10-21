#!/usr/bin/env python3
"""Compare creative marker selection and ambient sampling with the pinned ClientLevel."""
import itertools
import json

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab
    import oracle_harness as harness


def make_input(build):
    items = ['air', 'barrier', 'light', 'stone', 'structure_void', 'apple']
    cases = [dict(mode=mode, main=main, off=off, count=count)
             for mode, main, off, count in itertools.product(
                 ['survival', 'creative', 'adventure', 'spectator'], items, items, [0, 1, 64])]
    path = build / 'cases.jsonl'
    path.write_text(''.join(json.dumps(case) + '\n' for case in cases))
    return path, len(cases)


def main():
    harness.differential(
        'marker-particles-parity', 'VanillaMarkerParticlesOracle', 'marker_particles_probe',
        make_input, 'Creative main-hand marker selection and barrier marker positions, velocities and random draws',
        sources=[lab.ROOT / 'tools/test_marker_particles_parity.py',
                 lab.ROOT / 'crates/horizon-client/src/animate_tick.rs'], expected_lines=16872)


if __name__ == '__main__':
    main()
