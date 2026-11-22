#!/usr/bin/env python3
"""Compare item tooltip display visibility with the pinned vanilla client."""
import json
try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def cases(build):
    values = [
        (False, False, False),
        (False, False, True),
        (False, True, False),
        (False, True, True),
        (True, False, False),
        (True, False, True),
        (True, True, False),
        (True, True, True),
    ]
    path = build / 'cases.txt'
    # Lowercase: Rust's bool parser takes only true/false (Java's parseBoolean ignores case).
    names = [
        'Plain name',
        {'text': '', 'extra': [
            {'text': 'Red', 'color': '#ff0000', 'italic': False},
            {'text': 'Blue', 'color': '#0000ff'}]},
        {'text': 'Gold', 'color': 'gold', 'bold': True, 'italic': False},
        [{'text': 'List ', 'bold': True}, {'text': 'tail', 'bold': False}],
    ]
    jukebox_patches = [
        {},
        {'!minecraft:jukebox_playable': {}, 'minecraft:item_name': 'Test'},
        {'minecraft:jukebox_playable': 'minecraft:cat'},
        {'minecraft:tooltip_display': {'hidden_components': ['minecraft:jukebox_playable']}},
    ]
    rows = ['\t'.join(str(value).lower() for value in row) for row in values]
    rows.extend('name\t' + json.dumps(name, ensure_ascii=False) for name in names)
    for patch in jukebox_patches:
        disc = {'id': 'minecraft:music_disc_13', 'count': 1, 'components': patch}
        rows.append('jukebox\t' + json.dumps(disc))
    path.write_text('\n'.join(rows) + '\n', encoding='utf-8')
    return path, len(rows)


if __name__ == '__main__':
    harness.differential(
        'tooltip-display-parity', 'VanillaTooltipDisplayOracle', 'tooltip_display_fixture', cases,
        'Tooltip visibility, styled custom names and jukebox song component patches',
        sources=[
            lab.ROOT / 'crates/horizon-client/src/game/blocks/stations.rs',
            lab.ROOT / 'crates/horizon-client/src/item_tooltip.rs',
            lab.ROOT / 'crates/horizon-items/src/item_data.rs',
        ],
        expected_lines=16)
