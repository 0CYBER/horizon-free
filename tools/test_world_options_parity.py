#!/usr/bin/env python3
"""Compare World Options packet payloads with the pinned vanilla codecs."""
try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def cases(build):
    lines = [f'difficulty\t{i}' for i in range(4)] + ['lock\tfalse', 'lock\ttrue']
    for kind in ('set', 'values'):
        lines.append(kind)
        for name, values in [('keep_inventory', ['false', 'true']),
                             ('random_tick_speed', ['0', '3', '2147483647'])]:
            lines.extend(f'{kind}\t{name}\t{value}' for value in values)
    path = build / 'cases.txt'
    path.write_text('\n'.join(lines) + '\n')
    return path, len(lines)


if __name__ == '__main__':
    harness.differential(
        'world-options-parity', 'VanillaWorldOptionsOracle', 'world_options_fixture', cases,
        'ChangeDifficulty, LockDifficulty, SetGameRule and GameRuleValues payloads',
        sources=[lab.ROOT / 'crates/horizon-protocol/src/packets/play.rs'], expected_lines=18)
