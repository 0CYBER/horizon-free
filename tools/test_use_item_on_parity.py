#!/usr/bin/env python3
"""Compare a player's right click on a block with vanilla's, for items that are
both food and blocks (sweet and glow berries, carrots, potatoes) and for sweet
berry bushes.

Every case has a block at the origin, something above it, and a player three
blocks south, survival or creative, hungry or full, sneaking or not, who
right-clicks the top face of the origin holding an item. Vanilla runs the real
`ServerPlayerGameMode.useItemOn` over the headless level of the dispenser
harness (`VanillaDispenserOracle.useItem`): the block's `useItemOn` and
`useWithoutItem` (picking a bush's berries, `SweetBerryBushBlock.useItemOn`
passing bone meal on), then `ItemStack.useOn` (`BlockItem.place` with its
`mayPlaceOn` rules, `BoneMealItem.useOn`, and eating a consumable that could
not be placed). Horizon runs `use_item_as_player` on that hit in a headless
game (`HORIZON_DISPENSE_FIXTURE`). Both print block changes, dropped items,
level events, sounds, game events, the used statistic, the held stack, the
item the player started using, and the level random afterwards.
"""
import json
import os
import random
try:
    from . import block_update_harness as h, lab, test_dispenser_parity as d, test_rust_parity as base
except ImportError:
    import block_update_harness as h
    import lab
    import test_dispenser_parity as d
    import test_rust_parity as base

# Blocks the item is used on: the supports of the planted blocks and others.
GROUNDS = ['grass_block', 'dirt', 'coarse_dirt', 'podzol', 'rooted_dirt', 'moss_block', 'pale_moss_block', 'mud',
           'muddy_mangrove_roots', 'farmland', 'stone', 'sand', 'gravel', 'clay', 'snow_block', 'oak_planks']
ITEMS = ['sweet_berries', 'sweet_berries', 'sweet_berries', 'glow_berries', 'carrot', 'potato', 'bone_meal',
         'wheat_seeds', 'beetroot_seeds', 'apple']
# What is in the way above the clicked block.
ABOVE = ['air'] * 8 + ['short_grass', 'snow', 'stone', 'water', 'dead_bush', 'fern']


def cases(facts, count, seed):
    rng = random.Random(seed)
    states = d.States(facts)
    for n in range(count):
        case = {**h.environment(rng), 'pos': [0, 0, 0]}
        case.pop('raining', None)
        case.pop('thundering', None)
        cells = []
        item = rng.choice(ITEMS)
        if rng.random() < 0.3:
            # A sweet berry bush on grass: pick it, bone meal it, or plant against it.
            age = rng.randrange(4)
            cells.append([0, 0, 0, states.get('sweet_berry_bush', age=str(age))])
            cells.append([0, -1, 0, states.get(rng.choice(['grass_block', 'dirt', 'podzol', 'farmland']))])
            item = rng.choice(['bone_meal', 'bone_meal', 'sweet_berries', 'apple', '-', 'stick'])
        else:
            ground = rng.choice(GROUNDS)
            cells.append([0, 0, 0, states.any(rng, ground) if ground == 'farmland' else states.get(ground)])
            above = rng.choice(ABOVE)
            if above != 'air':
                cells.append([0, 1, 0, states.get(above)])
            if item == 'glow_berries' and rng.random() < 0.5:
                # A ceiling two blocks up for the cave vines to hang from.
                cells.append([0, 2, 0, states.get(rng.choice(['stone', 'dirt', 'oak_leaves']))])
            if item == 'bone_meal' and ground in ('grass_block', 'moss_block', 'pale_moss_block'):
                # Bone meal on spreading blocks is the bone meal harness's.
                item = 'sweet_berries'
        held = '-' if item == '-' else d.plain(item, rng.choice([1, 1, 2, 16, 64]))
        case['use_item'] = {'item': held, 'creative': rng.random() < 0.2, 'sneaking': rng.random() < 0.2,
                            'food': rng.choice([20, 20, 19, 10, 0])}
        case['cells'] = cells
        case['items'] = []
        case['mode'] = 'dispense'
        yield case


def kind(case, facts):
    first = facts.states[case['cells'][0][3]][0].split(':')[1]
    item = case['use_item']['item']
    item = item.split('"')[1].split(':')[1] if item != '-' else 'empty hand'
    return f'{item} on {first}'


def main():
    parser = h.argument_parser(__doc__)
    args = parser.parse_args()
    if args.count == 40000:
        args.count = 4000
    lock, jars, libraries = base.verify_artifacts()
    facts = h.load_facts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'use-item-on-parity'
    work.mkdir(parents=True, exist_ok=True)
    all_cases = list(cases(facts, args.count, args.seed))
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in all_cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    java_root = lab.ROOT / 'mods/horizon/src/test/java'
    d.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-sourcepath', str(java_root), '-nowarn',
           '-cp', dependencies, '-d', str(classes), str(java_root / 'dev/horizonlab/parity/VanillaDispenserOracle.java')])
    d.run(['java', '-Xmx3G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaDispenserOracle',
           str(resources), str(work / 'cases.jsonl'), str(work / 'native.txt')])
    d.run(['cargo', 'build', '-q', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
          env={**os.environ, 'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '3')})
    d.run([str(lab.cargo_target() / 'release/horizon-client'), '--resources', str(resources)], cwd=work,
          env={**os.environ, 'HORIZON_DISPENSE_FIXTURE': f'{work / "cases.jsonl"},{work / "rust.txt"}', 'HORIZON_NO_AUDIO': '1'})
    ignore = tuple(prefix + ' ' for prefix in args.ignore.split(',') if prefix) + ('D ',)
    native = h.split_cases((work / 'native.txt').read_text(), ignore)
    rust = h.split_cases((work / 'rust.txt').read_text(), ignore)
    assert len(native) == len(rust) == len(all_cases), (len(native), len(rust), len(all_cases))
    groups, errors = {}, {}
    planted = eaten = 0
    for case, a, b in zip(all_cases, native, rust):
        if a and a[0].startswith('error'):
            errors.setdefault(a[0], []).append(case)
            continue
        planted += any(line.startswith('set 0,1,0') for line in a)
        eaten += any(line.startswith('held') and not line.endswith('using -') for line in a)
        if not d.same(a, b):
            groups.setdefault(kind(case, facts), []).append((case, a, b))
    for name, entries in sorted(errors.items(), key=lambda e: -len(e[1])):
        print(f'oracle error x{len(entries)}: {name}')
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case ' + d.describe(case, facts) + ' use=' + json.dumps(case['use_item']))
            print('   native ' + ' | '.join(sorted(a)))
            print('   rust   ' + ' | '.join(sorted(b)))
    total = sum(len(v) for v in groups.values())
    skipped = sum(len(v) for v in errors.values())
    print(f'{len(all_cases) - total - skipped}/{len(all_cases) - skipped} cases match ({skipped} oracle errors); '
          f'vanilla planted in {planted} and started eating in {eaten}')
    h.finish(total + skipped)


if __name__ == '__main__':
    main()
