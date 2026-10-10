#!/usr/bin/env python3
"""Compare the data-driven axe/hoe/shovel block transformers with vanilla.

The Java oracle (VanillaBlockTransformerOracle) evaluates the registry's
BlockStateProviders on a stand-in level; the Rust example
(block_transform_fixture) evaluates horizon_client::block_transform. Cases are
every block state any transformer mentions (plus a random sample of all states)
under each transformer, with a few blocks above and three clicked faces.
"""
import argparse
import random
try:
    from . import interaction_lab as lib, lab
except ImportError:
    import interaction_lab as lib
    import lab

ABOVE = ['minecraft:air|', 'minecraft:cave_air|', 'minecraft:stone|', 'minecraft:water|level=0', 'minecraft:oak_slab|type=bottom,waterlogged=false']
FACES = ['up', 'down', 'north']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=7)
    parser.add_argument('--sample', type=int, default=3000)
    parser.add_argument('--limit', type=int, default=4)
    args = parser.parse_args()
    work = lib.work_dir('block-transformer-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaBlockTransformerOracle')
    vanilla.run('VanillaBlockTransformerOracle', 'dump', work / 'states.txt')
    states = (work / 'states.txt').read_text().splitlines()
    mentioned = set()
    for path in (lib.RESOURCES / 'data/minecraft/block_transformer').glob('*.json'):
        text = path.read_text()
        for name in set(__import__('re').findall(r'"minecraft:([a-z_0-9]+)"', text)):
            mentioned.add('minecraft:' + name)
    tags = set()
    for tag in ['turns_into_farmland', 'turns_into_dirt_path', 'air']:
        for name in __import__('json').loads((lib.RESOURCES / 'data/minecraft/tags/block' / (tag + '.json')).read_text())['values']:
            if not name.startswith('#'):
                tags.add(name)
    rng = random.Random(args.seed)
    chosen = [s for s in states if s.split('|')[0] in mentioned or s.split('|')[0] in tags or 'copper' in s or 'lightning_rod' in s]
    chosen += rng.sample(states, args.sample)
    cases = []
    for transformer in ['axe', 'hoe', 'shovel', 'honeycomb']:
        for state in chosen:
            for face in FACES:
                cases.append('\t'.join([transformer, face, state, rng.choice(ABOVE)]))
    (work / 'cases.txt').write_text(''.join(c + '\n' for c in cases))
    vanilla.run('VanillaBlockTransformerOracle', work / 'cases.txt', work / 'native.txt')
    lib.cargo_example('horizon-client', 'block_transform_fixture', lib.RESOURCES, work / 'cases.txt', work / 'rust.txt')
    total = lib.compare(cases, work / 'native.txt', work / 'rust.txt',
                        lambda case, line: case.split('\t')[0] + ' ' + line.split('\t')[0][:12], None, args.limit)
    raise SystemExit(1 if total else 0)


if __name__ == '__main__':
    main()
