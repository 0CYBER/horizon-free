#!/usr/bin/env python3
"""Compare horizon-world's level updateShape/canSurvive port with pinned vanilla.

Generates random 5x5x5 neighbourhoods around a random block state, evaluates
BlockState.updateShape towards one neighbour and canSurvive with the native
oracle (VanillaLevelShapeOracle) and with Behaviours (level_shape_fixture),
and reports every differing case.
"""
import argparse
import json
import os
import random
import subprocess
from pathlib import Path
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

# Blocks whose updateShape needs a full level (portal frames) are skipped.
SKIP = {'minecraft:nether_portal', 'minecraft:moving_piston'}


def generate(resources, count, seed):
    facts = json.loads((resources / 'facts.json').read_text())
    states = facts['states']
    by_block = {}
    for index, state in enumerate(states):
        by_block.setdefault(state[0], []).append(index)
    names = [name for name in by_block if name not in SKIP]
    common = [by_block[name][0] for name in ['minecraft:stone', 'minecraft:dirt', 'minecraft:grass_block', 'minecraft:water',
                                             'minecraft:oak_planks', 'minecraft:glass', 'minecraft:sand', 'minecraft:farmland']]
    rng = random.Random(seed)
    cases = []
    for _ in range(count):
        block = rng.choice(names)
        center = rng.choice(by_block[block])
        related = by_block[block]
        cells = []
        for x in range(-2, 3):
            for y in range(-2, 3):
                for z in range(-2, 3):
                    if (x, y, z) == (0, 0, 0):
                        cells.append([0, 0, 0, center])
                        continue
                    roll = rng.random()
                    if roll < 0.35:
                        continue
                    if roll < 0.55:
                        cells.append([x, y, z, rng.choice(common)])
                    elif roll < 0.75:
                        cells.append([x, y, z, rng.choice(related)])
                    else:
                        other = rng.choice(names)
                        cells.append([x, y, z, rng.choice(by_block[other])])
        cases.append({'seed': rng.randrange(1 << 40), 'light': rng.randrange(16), 'pos': [0, 0, 0],
                      'dir': rng.randrange(6), 'cells': cells, 'block': block})
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=20000)
    parser.add_argument('--seed', type=int, default=1)
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'level-shape-parity'
    work.mkdir(parents=True, exist_ok=True)
    cases = generate(resources, args.count, args.seed)
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    parity = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
    sources = [parity / 'VanillaLevelShapeOracle.java', parity / 'VanillaHeightmapOracle.java']
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', dependencies,
                    '-d', str(classes), *map(str, sources)], check=True)
    subprocess.run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies,
                    'dev.horizonlab.parity.VanillaLevelShapeOracle', str(resources), str(work / 'cases.jsonl'),
                    str(work / 'native.txt')], check=True)
    subprocess.run(['cargo', 'run', '-q', '-j2', '--release', '-p', 'horizon-world', '--example', 'world_probe', '--',
                    'level_shape_fixture', str(resources), str(work / 'cases.jsonl'), str(work / 'rust.txt')], check=True, cwd=lab.ROOT)
    native = (work / 'native.txt').read_text().splitlines()
    rust = (work / 'rust.txt').read_text().splitlines()
    differences = {}
    for case, a, b in zip(cases, native, rust):
        if a != b:
            differences.setdefault(case['block'], []).append((case, a, b))
    total = sum(len(v) for v in differences.values())
    for block, items in sorted(differences.items(), key=lambda item: -len(item[1])):
        case, a, b = items[0]
        print(f'{block}: {len(items)} differing; e.g. dir={case["dir"]}\n  native {a}\n  rust   {b}')
    print(f'{len(cases) - total}/{len(cases)} cases match')
    (work / 'differences.json').write_text(json.dumps({k: [[c, a, b] for c, a, b in v[:5]] for k, v in differences.items()}))
    raise SystemExit(1 if total else 0)


if __name__ == '__main__':
    main()
