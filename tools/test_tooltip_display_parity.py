#!/usr/bin/env python3
"""Compare item tooltip display visibility with the pinned vanilla client."""
try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def cases(build):
    values = [
        (False, False, False),
        (False, False, True),
        (False, True, True),
        (True, False, False),
        (True, True, False),
        (True, True, True),
    ]
    path = build / 'cases.txt'
    # Lowercase: Rust's bool parser takes only true/false (Java's parseBoolean ignores case).
    path.write_text('\n'.join('\t'.join(str(v).lower() for v in row) for row in values) + '\n')
    return path, len(values)


if __name__ == '__main__':
    harness.differential(
        'tooltip-display-parity', 'VanillaTooltipDisplayOracle', 'tooltip_display_fixture', cases,
        'Diamond sword name and attribute tooltip visibility for hide_tooltip, hidden_components, and creative flags',
        sources=[
            lab.ROOT / 'crates/horizon-client/src/game/stations.rs',
            lab.ROOT / 'crates/horizon-client/src/item_tooltip.rs',
        ],
        expected_lines=6)
