#!/usr/bin/env python3
"""Compare fully decorated regions around a player with vanilla.

Biome decoration is order dependent: a chunk's features write into its
neighbours, so where trees, ore blobs, sculk patches and geodes end up (and
whether a geode cuts through a stronghold room) depends on which chunk is
decorated first. test_chunk_generation_parity.py decorates row-major; this
harness decorates every site in the order vanilla's chunk scheduler uses
around a player standing in the site's chunk (horizon_world's
decoration_order::vanilla_order, which the game's streamer follows), on both
sides, and compares every block state, quart biome, block entity and
scheduled tick of the whole decorated area: the square of chunks whose 3x3
neighbourhood is decorated.

Sites: the spots of two reports on seed 1891623539983467248 (an old-growth
taiga forest at 204 108 -239; a stronghold with sculk and a geode at 4870 -22
401), a set of tree-heavy biomes on that seed, and the nearest mineshaft,
village, ancient city, trial chambers and stronghold-free structures found by
scanning it and a second seed.

usage: test_decorated_region_parity.py [--view N] [--filter REGEX] [--limit N]
"""

import argparse
import os
import re
import subprocess
import sys
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

try:
    from . import lab, test_chunk_generation_parity as cg
except ImportError:
    import lab
    import test_chunk_generation_parity as cg

REPORTS = 1891623539983467248
# (seed, label, chunk x, chunk z). Biome sites are /locate biome results.
SITES = [
    (REPORTS, 'report: old-growth taiga trees at 204 -239', 12, -15),
    (REPORTS, 'report: stronghold, sculk and geode at 4870 401', 304, 25),
    (REPORTS, 'taiga', 6, -10),
    (REPORTS, 'old_growth_spruce_taiga', -16, -50),
    (REPORTS, 'snowy_taiga', 54, -52),
    (REPORTS, 'dark_forest', -60, -96),
    (REPORTS, 'birch_forest', 8, 50),
    (REPORTS, 'jungle', -58, 48),
    (REPORTS, 'bamboo_jungle', -20, 70),
    (REPORTS, 'savanna', -28, 28),
    (REPORTS, 'cherry_grove', -10, -138),
    (REPORTS, 'mangrove_swamp', -4, 238),
    (REPORTS, 'pale_garden', -2, -202),
]
# Structures whose nearest start is added for each seed.
STRUCTURES = ['mineshaft', 'village', 'ancient_city', 'trial_chambers']
STRUCTURE_SEEDS = [REPORTS, 12345]
SCAN_RADIUS = 120


def build_dir():
    path = lab.LOCAL / 'decorated-region-parity'
    path.mkdir(parents=True, exist_ok=True)
    return path


def vanilla_order(binary, cx, cz, view):
    out = subprocess.run([str(binary), 'chunk_dump', str(cg.RESOURCES), '0', 'overworld'], capture_output=True, text=True, check=True,
                         env=dict(os.environ, HORIZON_LIST_ORDER=f'{cx},{cz},{view}'), cwd=lab.ROOT).stdout
    return [tuple(map(int, line.split())) for line in out.splitlines()]


def structure_sites(oracle, binary):
    sites = []
    for seed in STRUCTURE_SEEDS:
        starts = cg.scan(oracle, binary, seed, 'overworld', SCAN_RADIUS)
        for kind in STRUCTURES:
            found = sorted((abs(cx) + abs(cz), name, cx, cz) for name, cx, cz, _ in starts if kind in name)
            if found:
                _, name, cx, cz = found[0]
                sites.append((seed, name, cx, cz))
    return sites


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--view', type=int, default=4, help='view distance of the player at each site')
    parser.add_argument('--filter', help='regex over site labels')
    parser.add_argument('--limit', type=int, default=4, help='block differences shown per chunk')
    parser.add_argument('--no-structures', action='store_true', help='skip the scanned structure sites')
    args = parser.parse_args()
    os.environ['HORIZON_PHASE'] = 'full'
    oracle = cg.Oracle()
    binary = cg.rust_binary()
    names = cg.state_names()
    build = build_dir()
    sites = list(SITES) + ([] if args.no_structures else structure_sites(oracle, binary))
    if args.filter:
        sites = [s for s in sites if re.search(args.filter, s[1])]
    totals, started = Counter(), time.time()
    for seed, label, cx, cz in sites:
        # The player's tickets reach `view`; the chunks they get decorated reach
        # view + 3, so every chunk within view + 2 has its whole neighbourhood decorated.
        r = args.view + 2
        rect = f'{cx - r},{cz - r},{cx + r},{cz + r}'
        tag = f'{seed}-{cx}-{cz}-v{args.view}'
        order = build / f'order-{tag}.txt'
        order.write_text(''.join(f'{x} {z}\n' for x, z in vanilla_order(binary, cx, cz, args.view)))
        env = dict(os.environ, HORIZON_ORDER=str(order))
        vanilla_path, rust_path = build / f'vanilla-{tag}.txt', build / f'rust-{tag}.txt'
        t0 = time.time()
        saved = dict(os.environ)
        os.environ.update(env)
        try:
            with ThreadPoolExecutor(2) as pool:
                rust_run = pool.submit(cg.run_rust, binary, rust_path, seed, 'overworld', [rect])
                oracle.run(vanilla_path, ['dump', seed, 'overworld', rect])
                stderr = rust_run.result()
        finally:
            os.environ.clear()
            os.environ.update(saved)
        for line in stderr.splitlines()[:3]:
            print('  horizon:', line)
        vanilla, rust = cg.parse(vanilla_path), cg.parse(rust_path)
        counts = cg.compare(vanilla, rust, names, f'{label} ({seed})', limit=args.limit)
        totals.update(dict(zip(['chunks', 'blocks', 'biomes', 'entities'], counts)))
        totals['compared'] += len(vanilla)
        ticks = sum(len(c['t']) for c in vanilla.values())
        print(f'{label} (seed {seed}, chunk {cx},{cz}): {len(vanilla)} chunks, {ticks} scheduled ticks, differing chunks {counts[0]} '
              f'(blocks {counts[1]}, biome cells {counts[2]}, block entities {counts[3]}); {time.time() - t0:.0f}s', flush=True)
    print(f'Compared {totals["compared"]} chunks at {len(sites)} sites in {time.time() - started:.0f}s')
    print(f'Differences: {totals["chunks"]} chunks, {totals["blocks"]} blocks, {totals["biomes"]} biome cells, {totals["entities"]} block entities')
    if totals['chunks']:
        sys.exit(1)


if __name__ == '__main__':
    main()
