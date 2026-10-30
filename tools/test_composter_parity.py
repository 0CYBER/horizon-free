#!/usr/bin/env python3
"""Compare vanilla composters with Horizon's: by hand, through hoppers and from droppers.

Every case has a composter at a random level and runs one action over the
headless level of the dispenser harness (`VanillaDispenserOracle` against
`HORIZON_DISPENSE_FIXTURE`):

- `use`: a player (survival or creative) right-clicks the composter holding a
  compostable item, another item or nothing (`ComposterBlock.useItemOn`, then
  `useWithoutItem`, which empties a full composter);
- `hopper` above: a hopper facing down (or past it) pushes its items into the
  composter's input container;
- `hopper` below: a hopper under the composter pulls the bone meal out of its
  output container (nothing from the other levels);
- `dispense`: a dropper facing the composter puts one item in from above, or
  is refused from the side.

Both sides print block changes, container contents, spawned items, level
events with the sounds the client plays for them, sounds, game events, the
player's statistics and held stack, and the level random afterwards (the
compostable providers draw from it).
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

OTHERS = ['stick', 'stone', 'bone_meal', 'diamond_sword', 'water_bucket', 'cobblestone', 'egg']
SIDES = ['north', 'south', 'west', 'east']


def stack(rng, items, compostable, others_odds=0.25):
    if rng.random() < others_odds:
        name = rng.choice(OTHERS)
    else:
        name = rng.choice(compostable)
    top = items['minecraft:' + name].get('minecraft:max_stack_size', 64)
    return d.plain(name, min(top, rng.choice([1, 1, 2, 16, 64])))


def composter(states, rng):
    level = rng.choice([0, 0, 1, 3, 5, 6, 6, 7, 8, 8])
    return states.get('composter', level=str(level))


def cases(facts, items, count, seed):
    rng = random.Random(seed)
    states = d.States(facts)
    compostable = sorted(k.split(':')[1] for k, v in items.items() if 'minecraft:compostable' in v)
    modes = ['use', 'use', 'hopper_in', 'hopper_out', 'dispense']
    for n in range(count):
        mode = modes[n % len(modes)]
        case = {**h.environment(rng), 'pos': [0, 0, 0]}
        case.pop('raining', None)
        case.pop('thundering', None)
        cells = []
        items_in = []
        if mode == 'use':
            cells.append([0, 0, 0, composter(states, rng)])
            held = '-' if rng.random() < 0.1 else stack(rng, items, compostable)
            case['use'] = {'item': held, 'creative': rng.random() < 0.2}
        elif mode == 'hopper_in':
            facing = 'down' if rng.random() < 0.8 else rng.choice(SIDES)
            cells.append([0, 0, 0, states.get('hopper', facing=facing, enabled='true')])
            target = {'down': (0, -1, 0), 'north': (0, 0, -1), 'south': (0, 0, 1), 'west': (-1, 0, 0), 'east': (1, 0, 0)}[facing]
            cells.append([*target, composter(states, rng)])
            for slot in rng.sample(range(5), rng.choice([1, 1, 2, 3, 5])):
                items_in.append([0, 0, 0, slot, stack(rng, items, compostable, 0.4)])
            case['hopper'] = True
        elif mode == 'hopper_out':
            facing = rng.choice(['down', 'down'] + SIDES)
            cells.append([0, 0, 0, states.get('hopper', facing=facing, enabled='true')])
            level = 8 if rng.random() < 0.7 else rng.choice([0, 3, 7])
            cells.append([0, 1, 0, states.get('composter', level=str(level))])
            roll = rng.random()
            if roll < 0.3:
                # Full of something else: the bone meal can't go in.
                for slot in range(5):
                    items_in.append([0, 0, 0, slot, d.plain('stone', 64)])
            elif roll < 0.5:
                items_in.append([0, 0, 0, rng.randrange(5), d.plain('bone_meal', rng.choice([1, 63, 64]))])
            case['hopper'] = True
        else:
            facing = 'down' if rng.random() < 0.75 else rng.choice(SIDES + ['up'])
            cells.append([0, 0, 0, states.get('dropper', facing=facing, triggered='false')])
            dx, dy, dz = d.STEP[facing]
            cells.append([dx, dy, dz, composter(states, rng)])
            slots = rng.sample(range(9), rng.choice([1, 1, 2, 9]))
            for slot in slots:
                items_in.append([0, 0, 0, slot, stack(rng, items, compostable, 0.3)])
        case['cells'] = cells
        case['items'] = items_in
        case['mode'] = 'dispense'
        case['action'] = mode
        yield case


def main():
    parser = h.argument_parser(__doc__)
    args = parser.parse_args()
    if args.count == 40000:
        args.count = 3000
    lock, jars, libraries = base.verify_artifacts()
    facts = h.load_facts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    items = json.loads((resources / 'items.json').read_text())
    work = lab.LOCAL / 'composter-parity'
    work.mkdir(parents=True, exist_ok=True)
    all_cases = list(cases(facts, items, args.count, args.seed))
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in all_cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    java_root = lab.ROOT / 'mods/horizon/src/test/java'
    d.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-sourcepath', str(java_root), '-nowarn',
           '-cp', dependencies, '-d', str(classes), str(java_root / 'dev/horizonlab/parity/VanillaDispenserOracle.java')])
    d.run(['java', '-Xmx3G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaDispenserOracle',
           str(resources), str(work / 'cases.jsonl'), str(work / 'native.txt')])
    d.run(['cargo', 'build', '-q', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
          env={**os.environ, 'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '3')})
    d.run([str(lab.cargo_target() / 'fast/horizon-client'), '--resources', str(resources)], cwd=work,
          env=lab.game_env(HORIZON_DISPENSE_FIXTURE=f'{work / "cases.jsonl"},{work / "rust.txt"}'))
    ignore = tuple(prefix + ' ' for prefix in args.ignore.split(',') if prefix) + ('D ',)
    native = h.split_cases((work / 'native.txt').read_text(), ignore)
    rust = h.split_cases((work / 'rust.txt').read_text(), ignore)
    assert len(native) == len(rust) == len(all_cases), (len(native), len(rust), len(all_cases))
    groups, errors = {}, {}
    for case, a, b in zip(all_cases, native, rust):
        if a and a[0].startswith('error'):
            errors.setdefault(a[0], []).append(case)
            continue
        if not d.same(a, b):
            groups.setdefault(case['action'], []).append((case, a, b))
    for kind, entries in sorted(errors.items(), key=lambda e: -len(e[1])):
        print(f'oracle error x{len(entries)}: {kind}')
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case ' + d.describe(case, facts) + (' use=' + json.dumps(case['use']) if 'use' in case else ''))
            print('   native ' + ' | '.join(sorted(a)))
            print('   rust   ' + ' | '.join(sorted(b)))
    total = sum(len(v) for v in groups.values())
    skipped = sum(len(v) for v in errors.values())
    print(f'{len(all_cases) - total - skipped}/{len(all_cases) - skipped} cases match ({skipped} oracle errors)')
    h.finish(total + skipped)


if __name__ == '__main__':
    main()
