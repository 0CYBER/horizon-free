#!/usr/bin/env python3
"""Compare the blocks Horizon's structure mods place with the blocks a real server saved.

Expected side: the clean-room structure oracle (see test_structure_layout_parity.py), which kept, for every
start of a mod world, its blocks (`blocks`, the start's box) and every position of the box grown by
`diff_margin` where the mod world differs from the no-mod world of the same seed (`changed_vs_vanilla`).

Horizon side: the chunk_dump probe generates the chunks around each start in full (terrain, structures,
features) twice: from the 26.3 resources (the no-mod world) and from those resources with the mods carried
by `horizon-port-pack carry` (the mod world), as test_structure_layout_parity.py builds them.

The oracle's game is 1.21.1 and Horizon's 26.3, so the worlds also differ where the versions do. A position
is compared only where the two no-mod worlds agree (the oracle's no-mod block is known inside the start's
box and at the changed positions); elsewhere in the grown box, where the oracle saw no change, Horizon's mod
world must equal its no-mod world. Overlapping structures need no special case: both worlds have them.

A mismatch is counted by the oracle's no-mod block, the expected block and Horizon's, so a processor's
missing rule reads as one line. KNOWN names the mismatch classes the implementation does not cover yet, with
the reason; they are reported, not failed.

usage: test_structure_blocks_parity.py [--profile betterdungeons] [--filter REGEX] [--limit N]
"""
import argparse
import gzip
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict

try:
    from . import lab, test_structure_layout_parity as layout
except ImportError:
    import lab
    import test_structure_layout_parity as layout

# profile -> {structure id regex: what the implementation does not cover yet}
KNOWN = {
    'betterdungeons': {
        r'^betterdungeons:(small|skeleton|zombie)_dungeon$':
            "Better Dungeons' placeholder processors and its biome modifier that removes vanilla's "
            'monster rooms (spec-betterdungeons §1.2, §3.2) are not implemented yet',
        r'^betterdungeons:(spider|small_nether)_dungeon$':
            "Better Dungeons' own structure types (spec-betterdungeons §5, §6; OPEN-B14, B15) are not "
            'implemented yet',
    },
}


def state(text):
    """`name[k=v,...]` with sorted properties, the same for both games' spellings."""
    name, _, rest = text.partition('[')
    return name + ('[' + ','.join(sorted(rest.rstrip(']').split(','))) + ']' if rest else '')


def state_names(resources):
    facts = json.loads((resources / 'facts.json').read_text())
    return [state(s[3].removeprefix('Block{').replace('}', '', 1)) for s in facts['states']]


def chunk_rect(box):
    x0, _, z0, x1, _, z1 = box
    return f'{x0 >> 4},{z0 >> 4},{x1 >> 4},{z1 >> 4}'


def generate(binary, resources, seed, dimension, boxes, names):
    """Block states by position inside `boxes`, from chunk_dump's full generation of their chunks."""
    result = subprocess.run([str(binary), 'chunk_dump', str(resources), str(seed), dimension,
                             *(chunk_rect(b) for b in boxes)], capture_output=True, text=True, cwd=lab.ROOT)
    if result.returncode:
        raise SystemExit('chunk_dump failed: ' + result.stderr[-2000:])
    rows = {}  # chunk -> the heights the boxes over it need
    for x0, y0, z0, x1, y1, z1 in boxes:
        for cx in range(x0 >> 4, (x1 >> 4) + 1):
            for cz in range(z0 >> 4, (z1 >> 4) + 1):
                low, high = rows.get((cx, cz), (y0, y1))
                rows[(cx, cz)] = (min(low, y0), max(high, y1))
    blocks, chunk, low, high = {}, None, 0, -1
    for line in result.stdout.splitlines():
        if line.startswith('chunk '):
            _, x, z = line.split()
            low, high = rows.get((int(x), int(z)), (0, -1))
            chunk = (int(x) * 16, int(z) * 16)
        elif line.startswith('y '):
            _, y, row = line.split(' ', 2)
            y = int(y)
            if low <= y <= high:
                for i, state_id in enumerate(row.split(' ')):
                    blocks[(chunk[0] + i % 16, y, chunk[1] + i // 16)] = names[int(state_id)]
    return blocks


def oracle_blocks(start):
    """(mod world, no-mod world) block by position: the box in full, the changed positions around it."""
    x0, y0, z0, x1, y1, z1 = start['bounding_box']
    palette = [state(p) for p in start['blocks']['palette']]
    runs = iter(index for index, count in start['blocks']['rle'] for _ in range(count))
    mod = {(x, y, z): palette[next(runs)] for y in range(y0, y1 + 1) for z in range(z0, z1 + 1)
           for x in range(x0, x1 + 1)}
    base = dict(mod)
    for x, y, z, mod_state, base_state in start['changed_vs_vanilla']:
        mod[(x, y, z)], base[(x, y, z)] = state(mod_state), state(base_state)
    return mod, base


def inside(box, p):
    return all(box[i] <= p[i] <= box[i + 3] for i in range(3))


def compare(start, vanilla, modded):
    """Compared positions and the mismatches, as (no-mod, expected, Horizon) -> count."""
    mod, base = oracle_blocks(start)
    compared, mismatches = 0, Counter()
    x0, y0, z0, x1, y1, z1 = start['diff_box']
    for y in range(y0, y1 + 1):
        for z in range(z0, z1 + 1):
            for x in range(x0, x1 + 1):
                p = (x, y, z)
                if p not in vanilla or p not in modded:
                    continue
                if p in base:
                    if vanilla[p] != base[p]:
                        continue  # the versions differ here
                    expected = mod[p]
                else:
                    expected = vanilla[p]  # the oracle saw no change
                compared += 1
                if modded[p] != expected:
                    mismatches[(base.get(p, vanilla[p]), expected, modded[p])] += 1
    return compared, mismatches


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--profile', default='betterdungeons', choices=[p for p, v in layout.PROFILES.items() if v['carry']])
    parser.add_argument('--filter', default='', help='only structure ids matching this regex')
    parser.add_argument('--limit', type=int, default=8, help='mismatch classes shown per structure')
    args = parser.parse_args()
    profile = layout.PROFILES[args.profile]
    results = sorted((layout.ORACLE / args.profile).glob('seed-*.json.gz'))
    missing = [jar for jar in profile['mods'].values() if not (layout.JARS / jar).exists()]
    if not results or missing:
        print(f'SKIP: oracle results or mod jars missing under {layout.ORACLE} (see test_structure_layout_parity.py)')
        return 0
    resources = layout.overlay(args.profile, layout.carry(args.profile, profile['mods'], profile['carry']))
    binary = layout.probe_binary()
    names = state_names(lab.RESOURCES)
    totals = defaultdict(lambda: [0, 0, Counter()])  # structure -> [starts, compared, mismatches]
    for path in results:
        oracle = json.load(gzip.open(path))
        seed = oracle['seed']
        starts = [s for s in oracle['structures'] if s['complete'] and re.search(args.filter, s['id'])]
        for dimension in sorted({s['dimension'] for s in starts}):
            here = [s for s in starts if s['dimension'] == dimension]
            boxes = [s['diff_box'] for s in here]
            dim = dimension.removeprefix('minecraft:')
            vanilla = generate(binary, lab.RESOURCES, seed, dim, boxes, names)
            modded = generate(binary, resources, seed, dim, boxes, names)
            for start in here:
                compared, mismatches = compare(start, vanilla, modded)
                total = totals[start['id']]
                total[0] += 1
                total[1] += compared
                total[2].update(mismatches)
    failed = False
    for structure, (starts, compared, mismatches) in sorted(totals.items()):
        wrong = sum(mismatches.values())
        reason = next((r for pattern, r in KNOWN.get(args.profile, {}).items() if re.search(pattern, structure)), None)
        verdict = 'ok   ' if not wrong else 'known' if reason else 'FAIL '
        failed |= verdict == 'FAIL '
        print(f'{verdict} {structure}: {starts} starts, {compared - wrong} of {compared} positions match'
              + (f' ({reason})' if wrong and reason else ''))
        for (base, expected, got), n in mismatches.most_common(args.limit):
            print(f'        {n:6}  no-mod {base}  expected {expected}  Horizon {got}')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
