#!/usr/bin/env python3
"""Compare Horizon's structure starts, piece by piece, with the starts a real server saved.

Expected side: the clean-room structure oracle (horizon-parity `cleanroom/structures/oracle/`), which ran
vanilla 1.21.1 servers and kept every saved start. Its `base_pieces` are the no-mod world's start of each
located structure (villages, outposts, trial chambers, shipwrecks, ruined portals, desert pyramids): the
vanilla layout. Its results live outside the repository; put them (or link them) under
`.local/structure-oracle/<profile>/seed-<seed>.json.gz`, or point HORIZON_STRUCTURE_ORACLE at that folder.

Horizon side: the structure_pieces probe generates the same seed and lists the starts each chunk owns.
Jigsaw pieces must match exactly: element (template location), rotation, position, ground level delta and
bounding box, in saved order. Other pieces (shipwrecks, ruined portals, desert pyramids) match by type and
horizontal box: their height is settled when they are placed, after the start is created, and the oracle
saved them placed.

The oracle's game is 1.21.1 and Horizon's 26.3. KNOWN lists the starts where 26.3 itself differs from
1.21.1, each checked against 26.3 vanilla (VanillaChunkGenerationOracle's `scan`, HORIZON_PIECES=1), whose
pieces equal Horizon's there; they are reported, not failed.

Mod profiles compare the mod world's `pieces` instead. Horizon then reads the mod's own data from its jar
(`.local/structure-oracle/jars/`, or HORIZON_STRUCTURE_JARS): an overlay of the 26.3 resources adds the
jar's structures, sets, pools, processor lists, biome tags and templates, as they are written. The mod's
own processors are left out of the overlay's lists: they change blocks, never which pieces are placed.
Block states are upgraded to 26.3's JSON shape, as the port does.
The jars are the player's; nothing of them enters the repository.

usage: test_structure_layout_parity.py [--profile lithostitched|betterdungeons] [--filter REGEX] [--verbose]
"""
import argparse
import gzip
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ORACLE = Path(os.environ.get('HORIZON_STRUCTURE_ORACLE', lab.LOCAL / 'structure-oracle'))
JARS = Path(os.environ.get('HORIZON_STRUCTURE_JARS', ORACLE / 'jars'))
# Per profile: which saved pieces are expected, the mod jars whose data Horizon reads, and the structures
# compared (a regex; None: every start that has the pieces).
PROFILES = {
    'lithostitched': {'side': 'base_pieces', 'jars': [], 'structures': None},
    'betterdungeons': {'side': 'pieces', 'jars': ['YungsApi-1.21.1-NeoForge-5.1.9.jar', 'YungsBetterDungeons-1.21.1-NeoForge-5.1.4.jar'],
                       'structures': r'^betterdungeons:(small|skeleton|zombie)_dungeon$'},
}
BUNDLES = {'structure': 'structures', 'structure_set': 'structure_sets', 'template_pool': 'template_pools',
           'processor_list': 'processor_lists'}
# (profile, seed, structure id, start chunk x, z) -> how 26.3 vanilla differs from 1.21.1 there.
TRIAL_CHAMBERS = '26.3 vanilla lays it out as Horizon does, unlike 1.21.1'
SULFUR_CAVES = "the start is in 26.3's sulfur_caves, a biome the mod's 1.21.1 tag cannot list"
RUINED_PORTAL = "26.3 vanilla's portal is Horizon's; 1.21.1 picked another"
KNOWN = {
    ('lithostitched', 1234, 'minecraft:village_plains', -24, 47): 'no start in 26.3 vanilla (nor in Horizon)',
    ('lithostitched', 1234, 'minecraft:ruined_portal', -16, 51): RUINED_PORTAL,
    ('lithostitched', 42, 'minecraft:ruined_portal', -19, -38): RUINED_PORTAL,
        ('betterdungeons', 1234, 'betterdungeons:small_dungeon', 162, 10): SULFUR_CAVES,
    ('betterdungeons', 42, 'betterdungeons:small_dungeon', -29, -28): SULFUR_CAVES,
    **{('lithostitched', seed, 'minecraft:trial_chambers', x, z): TRIAL_CHAMBERS
       for seed, x, z in [(1234, 13, 21), (2, -28, 13), (2, 11, 16), (2, 146, 44), (42, 17, 7), (42, 38, -25)]},
}


def element(saved):
    kind = saved.get('element_type')
    if kind in ('minecraft:single_pool_element', 'minecraft:legacy_single_pool_element', 'yungsapi:yung_single_element'):
        location = saved['location']
        return location if ':' in location else 'minecraft:' + location
    if kind == 'minecraft:feature_pool_element':
        return 'feature:' + saved['feature']
    if kind == 'minecraft:list_pool_element':
        return 'list:' + '+'.join(element(e) for e in saved['elements'])
    if kind == 'minecraft:empty_pool_element':
        return 'empty'
    return kind


def expected_piece(saved):
    box = saved['BB']
    piece = {'type': saved['id'], 'box': box if saved['id'] == 'minecraft:jigsaw' else [box[0], box[2], box[3], box[5]]}
    if saved['id'] == 'minecraft:jigsaw':
        piece.update(element=element(saved['pool_element']), rotation=saved['rotation'],
                     position=[saved['PosX'], saved['PosY'], saved['PosZ']], ground_level_delta=saved['ground_level_delta'])
    return piece


def upgrade_states(value):
    """1.21.1's block states (`{"Name", "Properties"}`) in 26.3's shape (`{"id", "properties"}`)."""
    if isinstance(value, list):
        return [upgrade_states(v) for v in value]
    if not isinstance(value, dict):
        return value
    if 'Name' in value and set(value) <= {'Name', 'Properties'}:
        return {'id': value['Name'], **({'properties': value['Properties']} if 'Properties' in value else {})}
    return {k: upgrade_states(v) for k, v in value.items()}


def overlay(profile, jars):
    """The 26.3 resources plus the jars' structure data, in a folder the probe reads as its resources."""
    root = lab.LOCAL / 'structure-layout' / profile
    shutil.rmtree(root, ignore_errors=True)
    worldgen = root / 'worldgen'
    (worldgen / 'structure').mkdir(parents=True)
    (root / 'facts.json').symlink_to(lab.RESOURCES / 'facts.json')
    source = lab.RESOURCES / 'worldgen'
    merged = {name: json.loads((source / f'{name}.json').read_text()) for name in [*BUNDLES.values(), 'biome_tags']}
    for entry in source.iterdir():
        if entry.name != 'structure' and entry.stem not in merged:
            (worldgen / entry.name).symlink_to(entry)
    for namespace in (source / 'structure').iterdir():
        (worldgen / 'structure' / namespace.name).symlink_to(namespace)
    for jar in jars:
        with zipfile.ZipFile(JARS / jar) as archive:
            for name in archive.namelist():
                parts = name.split('/')
                if parts[0] != 'data' or len(parts) < 4 or name.endswith('/'):
                    continue
                namespace, kind = parts[1], parts[2]
                if kind == 'structure' and name.endswith('.nbt'):
                    target = worldgen / 'structure' / namespace / '/'.join(parts[3:])
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(archive.read(name))
                elif name.endswith('.json') and kind == 'worldgen' and parts[3] in BUNDLES:
                    value = upgrade_states(json.loads(archive.read(name)))
                    if parts[3] == 'processor_list':
                        value['processors'] = [p for p in value['processors'] if p['processor_type'].startswith('minecraft:')]
                    merged[BUNDLES[parts[3]]][f'{namespace}:' + '/'.join(parts[4:])[:-5]] = json.dumps(value)
                elif name.endswith('.json') and parts[2:5] == ['tags', 'worldgen', 'biome']:
                    tag = f'{namespace}:' + '/'.join(parts[5:])[:-5]
                    value = json.loads(archive.read(name))
                    if tag in merged['biome_tags'] and not value.get('replace'):
                        value['values'] = json.loads(merged['biome_tags'][tag])['values'] + value['values']
                    merged['biome_tags'][tag] = json.dumps(value)
    for name, bundle in merged.items():
        (worldgen / f'{name}.json').write_text(json.dumps(bundle))
    return root


def probe_binary():
    subprocess.run(['cargo', 'build', '--release', '-q', '-p', 'horizon-world', '--example', 'world_probe'], check=True, cwd=lab.ROOT)
    name = 'world_probe.exe' if os.name == 'nt' else 'world_probe'
    return lab.cargo_target() / 'release/examples' / name


def horizon_starts(binary, resources, seed, dimension, chunks):
    result = subprocess.run([str(binary), 'structure_pieces', str(resources), str(seed), dimension,
                             *(f'{x},{z}' for x, z in chunks)], capture_output=True, text=True, cwd=lab.ROOT)
    if result.returncode:
        raise SystemExit('structure_pieces failed: ' + result.stderr[-2000:])
    starts, current = {}, None
    for line in result.stdout.splitlines():
        fields = line.split()
        if fields[0] == 'start':
            current = starts.setdefault((fields[1], int(fields[2]), int(fields[3])), [])
        elif fields[0] == 'piece':
            box = list(map(int, fields[8:14]))
            piece = {'type': fields[1], 'box': box if fields[1] == 'minecraft:jigsaw' else [box[0], box[2], box[3], box[5]]}
            if fields[1] == 'minecraft:jigsaw':
                piece.update(element=fields[2], rotation=fields[3], position=list(map(int, fields[4:7])),
                             ground_level_delta=int(fields[7]))
            current.append(piece)
    return starts


def compare(expected, actual):
    """Index of the first differing piece and what differs, or None."""
    for i, (want, got) in enumerate(zip(expected, actual)):
        if want != got:
            return i, {k: (want.get(k), got.get(k)) for k in want.keys() | got.keys() if want.get(k) != got.get(k)}
    if len(expected) != len(actual):
        return min(len(expected), len(actual)), {'pieces': (len(expected), len(actual))}
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--profile', default='lithostitched', choices=sorted(PROFILES))
    parser.add_argument('--filter', default='', help='only structure ids matching this regex')
    parser.add_argument('--verbose', action='store_true')
    args = parser.parse_args()
    results = sorted((ORACLE / args.profile).glob('seed-*.json.gz'))
    if not results:
        print(f'SKIP: no oracle results in {ORACLE / args.profile} (see this file\'s docstring)')
        return 0
    profile = PROFILES[args.profile]
    missing = [jar for jar in profile['jars'] if not (JARS / jar).exists()]
    if missing:
        print(f'SKIP: mod jars not in {JARS}: {", ".join(missing)}')
        return 0
    resources = overlay(args.profile, profile['jars']) if profile['jars'] else lab.RESOURCES
    binary = probe_binary()
    matched, known, failed = 0, 0, []
    for path in results:
        oracle = json.load(gzip.open(path))
        seed = oracle['seed']
        by_dimension = {}
        for start in oracle['structures']:
            if start.get(profile['side']) is None or not re.search(args.filter, start['id']) \
                    or not re.search(profile['structures'] or '', start['id']):
                continue
            by_dimension.setdefault(start['dimension'], []).append(start)
        for dimension, starts in by_dimension.items():
            chunks = sorted({tuple(s['start_chunk']) for s in starts})
            actual = horizon_starts(binary, resources, seed, dimension.removeprefix('minecraft:'), chunks)
            for start in starts:
                key = (start['id'], *start['start_chunk'])
                expected = [expected_piece(p) for p in start[profile['side']]]
                got = actual.get(key)
                label = f'seed {seed} {start["id"]} chunk {start["start_chunk"]}'
                difference = (0, {'start': 'missing in Horizon'}) if got is None else compare(expected, got)
                reason = KNOWN.get((args.profile, seed, *key))
                if difference is None:
                    matched += 1
                    if args.verbose:
                        print(f'ok    {label}: {len(expected)} pieces')
                elif reason:
                    known += 1
                    print(f'known {label}: {reason}')
                else:
                    failed.append(label)
                    index, fields = difference
                    print(f'FAIL  {label}: {len(expected)} pieces expected, {len(got or [])} placed; piece {index}: {fields}')
    total = matched + known + len(failed)
    print(f'{matched} of {total} starts match the oracle piece for piece; {known} known version differences')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
