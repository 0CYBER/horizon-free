#!/usr/bin/env python3
"""Compare horizon-world's placement port (BlockItem placement state) with pinned vanilla.

Generates random neighbourhoods and use-item-on-block actions for random block
items, resolves the placed position and state with the native oracle
(VanillaBlockPlacementOracle) and with Behaviours (level_place_fixture), and reports
differing cases grouped by item.
"""
import argparse
import json
import os
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

# Items whose placement reads block-entity data or entities.
SKIP = {'minecraft:lectern', 'minecraft:test_block', 'minecraft:moving_piston'}


def generate(resources, count, seed):
    facts = json.loads((resources / 'facts.json').read_text())
    behaviours = json.loads((resources / 'block-behaviours.json').read_text())
    by_block = {}
    for index, state in enumerate(facts['states']):
        by_block.setdefault(state[0], []).append(index)
    items = sorted(name for name, item in behaviours['items'].items() if name not in SKIP and item['fields'].get('block') in by_block)
    names = list(by_block)
    common = [by_block[name][0] for name in ['minecraft:stone', 'minecraft:dirt', 'minecraft:grass_block', 'minecraft:water',
                                             'minecraft:oak_planks', 'minecraft:glass', 'minecraft:sand', 'minecraft:oak_log']]
    rng = random.Random(seed)
    cases = []
    for _ in range(count):
        item = rng.choice(items)
        block = behaviours['items'][item]['fields']['block']
        related = by_block[block]
        cells = []
        for x in range(-2, 3):
            for y in range(-2, 3):
                for z in range(-2, 3):
                    roll = rng.random()
                    if (x, y, z) == (0, 0, 0):
                        roll = rng.random() * 1.2
                    if roll < 0.35:
                        continue
                    if roll < 0.6:
                        cells.append([x, y, z, rng.choice(common)])
                    elif roll < 0.8:
                        cells.append([x, y, z, rng.choice(related)])
                    else:
                        cells.append([x, y, z, rng.choice(by_block[rng.choice(names)])])
        face = rng.randrange(6)
        normal = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)][face]
        hit = []
        for axis in range(3):
            if normal[axis] < 0:
                hit.append(0.0)
            elif normal[axis] > 0:
                hit.append(1.0)
            else:
                hit.append(rng.choice([0.5, rng.random()]))
        cases.append({'seed': rng.randrange(1 << 40), 'item': item, 'pos': [0, 0, 0], 'face': face, 'hit': hit,
                      'yaw': rng.randrange(-180 * 64, 180 * 64) / 64, 'pitch': rng.randrange(-90 * 64, 90 * 64) / 64,
                      'sneak': rng.random() < 0.2, 'cells': cells})
    return cases


def obstruction_cases(resources):
    """Placement failures must not predict a hand swing.

    Exercise BlockItem.canPlace's entity-obstruction gate with the same
    otherwise valid placements, including partial shapes and two-block items.
    """
    facts = json.loads((resources / 'facts.json').read_text())
    states = {s[0]: i for i, s in reversed(list(enumerate(facts['states'])))}
    stone = states['minecraft:stone']
    for item in ['minecraft:stone', 'minecraft:oak_slab', 'minecraft:oak_stairs',
                 'minecraft:oak_door', 'minecraft:torch']:
        for obstructed in [False, True]:
            yield dict(seed=0, item=item, pos=[0, 0, 0], face=1, hit=[0.5, 1.0, 0.5],
                       yaw=0, pitch=45, sneak=False, cells=[[0, 0, 0, stone]], obstructed=obstructed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=20000)
    parser.add_argument('--seed', type=int, default=1)
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'level-place-parity'
    work.mkdir(parents=True, exist_ok=True)
    cases = list(obstruction_cases(resources)) + generate(resources, args.count, args.seed)
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    parity = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
    sources = [parity / 'VanillaBlockPlacementOracle.java', parity / 'VanillaHeightmapOracle.java', parity / 'VanillaLevelShapeOracle.java']
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-nowarn', '-cp', dependencies,
                    '-d', str(classes), *map(str, sources)], check=True)
    subprocess.run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies,
                    'dev.horizonlab.parity.VanillaBlockPlacementOracle', str(resources), str(work / 'cases.jsonl'),
                    str(work / 'native.txt')], check=True)
    subprocess.run(['cargo', 'run', '-q', '-j2', '--release', '-p', 'horizon-world', '--example', 'world_probe', '--',
                    'level_place_fixture', str(resources), str(work / 'cases.jsonl'), str(work / 'rust.txt')], check=True, cwd=lab.ROOT)
    native = (work / 'native.txt').read_text().splitlines()
    rust = (work / 'rust.txt').read_text().splitlines()
    differences = {}
    for case, a, b in zip(cases, native, rust):
        if a != b:
            differences.setdefault(case['item'], []).append((case, a, b))
    total = sum(len(v) for v in differences.values())
    facts = json.loads((resources / 'facts.json').read_text())['states']
    def describe(line):
        parts = line.split()
        return line if len(parts) < 3 or not parts[2].isdigit() else f'{parts[1]} {facts[int(parts[2])][3]}'
    for item, entries in sorted(differences.items(), key=lambda entry: -len(entry[1])):
        case, a, b = entries[0]
        print(f'{item}: {len(entries)} differing; e.g. face={case["face"]} yaw={case["yaw"]} pitch={case["pitch"]} sneak={case["sneak"]}'
              f'\n  native {describe(a)}\n  rust   {describe(b)}')
    print(f'{len(cases) - total}/{len(cases)} cases match')
    (work / 'differences.json').write_text(json.dumps({k: [[c, a, b] for c, a, b in v[:5]] for k, v in differences.items()}))
    raise SystemExit(1 if total else 0)


if __name__ == '__main__':
    main()
