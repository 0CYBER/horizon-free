#!/usr/bin/env python3
"""A block entity changed by a player's hand wakes the comparator reading it, tick by tick, as in vanilla.

Vanilla commits a block entity change with one call, `BlockEntity.setChanged`, which marks the chunk
unsaved and, through `Level.updateNeighbourForOutputSignal`, makes the comparators beside the block
(or behind a solid block beside it) re-read its `getAnalogOutputSignal`. Horizon splits that into
`container_changed` (wakes comparators) and `mark_block_entity_updated` (does not), paired by hand at
each write path, and a missed pairing shipped three times (#1831, #2218, #18082). This fixture can see
that bug: a survival player fills a decorated pot by hand through each game's real use path (the
connected owner's `UseItemOn` packet, acknowledged, in Horizon; `ServerPlayerGameMode.useItemOn` in
vanilla), and after every tick both games report the comparator's `OutputSignal`, its `powered`
state, the pot's item and the held stack. A pot write that skips the wake leaves the comparator on
its old strength, which differs from vanilla two ticks later.

Cases, each with blocks of its own and a settled comparator before the use: an empty pot gets one
snowball (strength 0 -> 1); a pot of 14 snowballs gets its 15th (13 -> 14, snowballs stack to 16);
a full pot of 16 and a pot given another item (an egg) refuse the item and stay as they were; and a
comparator reads an empty pot through a stone block (0 -> 1). Vanilla's strengths before and after
each use are fixture guards: a case whose strength doesn't move in vanilla stops the run instead of
passing for nothing.

Runs through tools/test_command_exec_parity.py (`--groups-file`): its common setup, then this group.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

try:
    from .command_exec_corpus import cmd, setup
except ImportError:
    from command_exec_corpus import cmd, setup

GROUP = 'block entity change contract'
PLAYER = (6.5, 200, 6.5)
TICKS_AFTER_USE = 4
# (name, pot, comparator facing the pot, solid block between them, saved pot item, held item, strength
# before, strength after). A comparator faces the block it reads (DiodeBlock.getInputSignal).
CASES = [
    ('empty pot gets one', (4, 200, 4), (4, 200, 3, 'south'), None, None, ('snowball', 2), 0, 1),
    ('fourteen to fifteen', (8, 200, 4), (8, 200, 3, 'south'), None, ('snowball', 14), ('snowball', 2), 13, 14),
    ('full pot refuses', (4, 200, 9), (4, 200, 10, 'north'), None, ('snowball', 16), ('snowball', 2), 15, 15),
    ('other item refused', (8, 200, 9), (8, 200, 10, 'north'), None, ('snowball', 14), ('egg', 1), 13, 13),
    ('through a solid block', (6, 200, 3), (6, 200, 1, 'south'), (6, 200, 2), None, ('snowball', 2), 0, 1),
]


def at(pos):
    return ' '.join(map(str, pos[:3]))


def observe(case):
    """What both games report after a tick: the comparator's strength and state, the pot, the hand."""
    _, pot, comparator, _, _, (held, _), _, _ = case
    return [cmd(f'/data get block {at(comparator)} OutputSignal'),
            cmd(f'/execute if block {at(comparator)} minecraft:comparator[powered=true]'),
            cmd(f'/data get block {at(pot)} item'),
            cmd(f'/execute if items entity @s weapon.mainhand minecraft:{held}')]


def case_ops(case, sequence):
    name, pot, comparator, solid, saved, (held, count), before, after = case
    item = f'{{item:{{id:"minecraft:{saved[0]}",count:{saved[1]}}}}}' if saved else ''
    # The comparator learns of its input from a neighbour update, never from its own /setblock: the pot
    # placed beside it, or the solid block placed last in front of it.
    ops = setup(f'/setblock {at(comparator)} minecraft:comparator[facing={comparator[3]}]',
                f'/setblock {at(pot)} minecraft:decorated_pot{item}')
    if solid:
        ops += setup(f'/setblock {at(solid)} minecraft:stone')
    ops += setup(f'/item replace entity @s weapon.mainhand with minecraft:{held} {count}')
    ops.append({'op': 'tick', 'n': 4})
    ops.append(cmd(f'/data get block {at(comparator)} OutputSignal', expect_result=before))
    ops += observe(case)[1:]
    ops.append({'op': 'use_block', 'pos': list(pot), 'face': 1, 'sequence': sequence})
    ops += observe(case)
    for tick in range(TICKS_AFTER_USE):
        ops.append({'op': 'tick', 'n': 1})
        last = tick == TICKS_AFTER_USE - 1
        ops.append(cmd(f'/data get block {at(comparator)} OutputSignal', expect_result=after if last else None))
        ops += observe(case)[1:]
    return ops


def groups():
    ops = setup('/fill 0 199 0 12 199 12 minecraft:stone', '/fill 0 200 0 12 203 12 minecraft:air',
                '/gamemode survival', f'/tp @s {PLAYER[0]} {PLAYER[1]} {PLAYER[2]} 0 90')
    ops.append({'op': 'settle'})
    ops.append(cmd(f'/execute if entity @s[gamemode=survival,nbt={{Pos:[{PLAYER[0]}d,{PLAYER[1]}d,{PLAYER[2]}d]}}]',
                   expect_result=1))
    for sequence, case in enumerate(CASES, start=1):
        ops += case_ops(case, sequence)
    ops += setup('/fill 0 200 0 12 203 12 minecraft:air', '/clear @s', '/gamemode creative', '/tp @s 8.5 230 8.5 0 0')
    ops.append({'op': 'settle'})
    return [(GROUP, ops)]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, help='an already built Horizon game')
    parser.add_argument('--reference-only', action='store_true', help='run and admit only the vanilla side')
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    work = (args.work_dir or root / '.local' / 'block-entity-change-contract-parity').resolve()
    work.mkdir(parents=True, exist_ok=True)
    cases = work / 'groups.json'
    cases.write_text(json.dumps(groups()), encoding='utf-8')
    run = [sys.executable, str(root / 'tools/test_command_exec_parity.py'),
           '--groups-file', str(cases), '--work-dir', str(work)]
    if args.binary:
        run.extend(['--binary', str(args.binary.resolve())])
    if args.reference_only:
        run.append('--reference-only')
    return subprocess.run(run, cwd=root).returncode


if __name__ == '__main__':
    sys.exit(main())
