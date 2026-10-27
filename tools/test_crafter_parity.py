#!/usr/bin/env python3
"""Compare vanilla's crafter with Horizon's, one redstone-triggered craft per case.

A crafter at the origin, in one of its twelve orientations, holds a grid
filled from a random vanilla crafting recipe (any recipe of the datapack, the
cake and other recipes with remainders, special recipes, or junk that matches
nothing), with some empty slots disabled. In front stands a container (chest,
hopper, barrel, dropper, furnace, shulker box, another crafter with its own
grid and disabled slots, sometimes nearly full), a solid block, water or air.
Vanilla `CrafterBlock.dispenseFrom` runs over the headless level of the block
update oracle (`VanillaDispenserOracle`), followed by a few
`CrafterBlockEntity.serverTick`s; Horizon runs its game glue in a headless
game (`HORIZON_DISPENSE_FIXTURE`). Both print block changes (the crafting
flag set and cleared), every container's contents, each crafter's animation
ticks, triggered flag, comparator signal and disabled slots, the items shot
out with their motion, level events, sounds and the level random.
"""
import json
import os
import random
import subprocess
try:
    from . import block_update_harness as h, lab, test_dispenser_parity as d, test_rust_parity as base
except ImportError:
    import block_update_harness as h
    import lab
    import test_dispenser_parity as d
    import test_rust_parity as base

FRONTS = ['north_up', 'south_up', 'west_up', 'east_up', 'up_north', 'up_south', 'up_west', 'up_east',
          'down_north', 'down_south', 'down_west', 'down_east']
STEP = {'down': (0, -1, 0), 'up': (0, 1, 0), 'north': (0, 0, -1), 'south': (0, 0, 1), 'west': (-1, 0, 0), 'east': (1, 0, 0)}
FRONT_BLOCKS = ['air', 'air', 'air', 'chest', 'chest', 'hopper', 'barrel', 'dropper', 'dispenser', 'crafter', 'crafter',
                'furnace', 'blast_furnace', 'smoker', 'shulker_box', 'stone', 'water', 'trapped_chest', 'brewing_stand']
# Recipes whose remainders the crafter shoots out too, and specials.
FAVOURITES = ['cake', 'honey_block', 'sugar_from_honey_bottle', 'firework_rocket_simple', 'map_cloning', 'book_cloning',
              'oak_planks', 'stick', 'crafting_table', 'chest', 'torch', 'iron_block', 'iron_ingot_from_iron_block',
              'bread', 'tnt', 'white_banner', 'shield_decoration', 'armor_dye', 'repair_item', 'tipped_arrow',
              'suspicious_stew', 'decorated_pot', 'bundle', 'white_bed', 'red_dye_from_poppy', 'mushroom_stew',
              'pumpkin_pie', 'rabbit_stew_from_brown_mushroom', 'beacon', 'oak_boat', 'dye_red_wool']
JUNK = ['dirt', 'stone', 'diamond', 'stick', 'oak_planks', 'apple', 'feather', 'gold_nugget', 'bone', 'iron_ingot']


def item(name, count=1, extra=''):
    return '{id:"minecraft:%s",count:%d%s}' % (name, count, extra)


class Recipes:
    """Shaped and shapeless crafting recipes of the datapack, with item tags resolved."""

    def __init__(self, resources):
        root = resources / 'data/minecraft'
        self.recipes = {}
        for path in sorted((root / 'recipe').glob('*.json')):
            self.recipes[path.stem] = json.loads(path.read_text())
        self.tag_dir = root / 'tags/item'
        self.tags = {}

    def tag(self, name):
        if name not in self.tags:
            path = self.tag_dir / (name.split(':')[1] + '.json')
            out = []
            for v in json.loads(path.read_text())['values'] if path.exists() else []:
                v = v['id'] if isinstance(v, dict) else v
                out.extend(self.tag(v[1:]) if v.startswith('#') else [v])
            self.tags[name] = out
        return self.tags[name]

    def choices(self, ingredient):
        if isinstance(ingredient, list):
            out = []
            for i in ingredient:
                out.extend(self.choices(i))
            return out
        if isinstance(ingredient, dict):
            ingredient = ingredient.get('item') or ('#' + ingredient['tag'])
        return self.tag(ingredient[1:]) if ingredient.startswith('#') else [ingredient]

    def grid(self, rng, name):
        """The nine cells of one way to fill the recipe, or None for specials without a pattern."""
        r = self.recipes.get(name)
        if r is None:
            return None
        cells = [None] * 9
        if r['type'] == 'minecraft:crafting_shaped':
            pattern = r['pattern']
            if len(pattern) > 3 or any(len(row) > 3 for row in pattern):
                return None
            dx = rng.randrange(0, 4 - max(len(row) for row in pattern))
            dy = rng.randrange(0, 4 - len(pattern))
            for y, row in enumerate(pattern):
                for x, ch in enumerate(row):
                    if ch != ' ':
                        cells[(y + dy) * 3 + x + dx] = rng.choice(self.choices(r['key'][ch])).split(':')[1]
        elif r['type'] == 'minecraft:crafting_shapeless':
            ingredients = r['ingredients']
            if len(ingredients) > 9:
                return None
            for at, ing in zip(rng.sample(range(9), len(ingredients)), ingredients):
                cells[at] = rng.choice(self.choices(ing)).split(':')[1]
        elif r['type'] == 'minecraft:crafting_transmute':
            at = rng.sample(range(9), 2)
            cells[at[0]] = rng.choice(self.choices(r['input'])).split(':')[1]
            cells[at[1]] = rng.choice(self.choices(r['material'])).split(':')[1]
        else:
            return None
        return cells


SPECIAL = {
    'firework_rocket_simple': lambda rng: ['paper', 'gunpowder'] + ['gunpowder'] * rng.randrange(0, 3),
    'map_cloning': lambda rng: ['filled_map', 'map'],
    'book_cloning': lambda rng: ['written_book', 'writable_book'],
    'armor_dye': lambda rng: ['leather_helmet', rng.choice(['red_dye', 'blue_dye', 'white_dye'])],
    'repair_item': lambda rng: ['iron_pickaxe', 'iron_pickaxe'],
    'shield_decoration': lambda rng: ['shield', 'red_banner'],
    'suspicious_stew': lambda rng: ['bowl', 'red_mushroom', 'brown_mushroom', 'poppy'],
    'tipped_arrow': lambda rng: None,
}


def fill_special(rng, name):
    make = SPECIAL.get(name)
    items = make(rng) if make else None
    if not items:
        return None
    cells = [None] * 9
    for at, it in zip(rng.sample(range(9), len(items)), items):
        cells[at] = it
    return cells


def stack_of(name, count, rng):
    extra = ''
    if name in ('iron_pickaxe',):
        extra = ',components:{"minecraft:damage":%d}' % rng.randrange(1, 200)
    if name == 'written_book':
        extra = ',components:{"minecraft:written_book_content":{title:{raw:"t"},author:"a",generation:0}}'
    if name == 'filled_map':
        extra = ',components:{"minecraft:map_id":%d}' % rng.randrange(0, 5)
    return item(name, count, extra)


def counts(rng, name, items):
    top = items.get('minecraft:' + name, {}).get('minecraft:max_stack_size', 64)
    if top == 1:
        return 1
    return min(top, rng.choice([1, 1, 1, 2, 3, 5, top]))


def cases(facts, items, recipes, count, seed):
    rng = random.Random(seed)
    states = d.States(facts)
    names = [n for n in recipes.recipes if recipes.recipes[n]['type'] in
             ('minecraft:crafting_shaped', 'minecraft:crafting_shapeless', 'minecraft:crafting_transmute')]
    for n in range(count):
        orientation = rng.choice(FRONTS)
        front_dir = orientation.split('_')[0]
        triggered = rng.random() < 0.9
        crafting = rng.random() < 0.2
        cells = [[0, 0, 0, states.get('crafter', orientation=orientation, triggered=str(triggered).lower(),
                                      crafting=str(crafting).lower())]]
        roll = rng.random()
        if roll < 0.45:
            name = rng.choice(names)
        elif roll < 0.75:
            name = rng.choice(FAVOURITES)
        else:
            name = None
        grid = None
        if name:
            grid = recipes.grid(rng, name) or fill_special(rng, name)
        if grid is None:
            grid = [rng.choice(JUNK) if rng.random() < 0.4 else None for _ in range(9)]
        if rng.random() < 0.1:
            # A stray item that spoils the recipe.
            grid[rng.randrange(9)] = rng.choice(JUNK)
        stacks = []
        for slot, cell in enumerate(grid):
            if cell:
                stacks.append([0, 0, 0, slot, stack_of(cell, counts(rng, cell, items), rng)])
        disabled = []
        empty = [i for i in range(9) if not grid[i]]
        for i in rng.sample(empty, rng.randrange(0, len(empty) + 1)) if empty and rng.random() < 0.4 else []:
            disabled.append([0, 0, 0, i])
        if rng.random() < 0.1 and stacks:
            # Disabling a filled slot does nothing.
            disabled.append([0, 0, 0, rng.choice(stacks)[3]])
        dx, dy, dz = STEP[front_dir]
        front = (dx, dy, dz)
        block = rng.choice(FRONT_BLOCKS)
        if block == 'crafter':
            h.put(cells, front, states.get('crafter', orientation=rng.choice(FRONTS), triggered='false', crafting='false'))
            for i in range(9):
                roll = rng.random()
                if roll < 0.35:
                    stacks.append([dx, dy, dz, i, item(rng.choice(['oak_planks', 'stick', 'cake', 'bucket', 'glass_bottle', 'stone']),
                                                       rng.choice([1, 1, 2, 63, 64]))])
                elif roll < 0.5:
                    disabled.append([dx, dy, dz, i])
        elif block in ('chest', 'trapped_chest', 'furnace', 'blast_furnace', 'smoker', 'dropper', 'dispenser', 'hopper', 'barrel', 'brewing_stand'):
            props = {}
            if block in ('chest', 'trapped_chest', 'furnace', 'blast_furnace', 'smoker'):
                props['facing'] = rng.choice(['north', 'south', 'west', 'east'])
            h.put(cells, front, states.get(block, **props))
            size = {'hopper': 5, 'dropper': 9, 'dispenser': 9, 'furnace': 3, 'blast_furnace': 3, 'smoker': 3, 'brewing_stand': 5}.get(block, 27)
            fill = rng.choice([0, 0, 0.3, 0.95, 1.0])
            for i in range(size):
                if rng.random() < fill:
                    stacks.append([dx, dy, dz, i, item(rng.choice(['stone', 'oak_planks', 'cake', 'stick', 'bucket', 'diamond']),
                                                       rng.choice([1, 16, 63, 64, 64]))])
        elif block == 'shulker_box':
            h.put(cells, front, states.get('shulker_box', facing=front_dir))
        elif block != 'air':
            h.put(cells, front, states.get(block))
        case = {**h.environment(rng), 'mode': 'craft', 'pos': [0, 0, 0], 'cells': cells, 'items': stacks,
                'disabled': disabled, 'ticks': rng.choice([0, 0, 1, 3, 5, 6, 7, 9])}
        case.pop('raining', None)
        case.pop('thundering', None)
        yield case


def main():
    parser = h.argument_parser(__doc__)
    args = parser.parse_args()
    if args.count == 40000:
        args.count = 4000
    lock, jars, libraries = base.verify_artifacts()
    facts = h.load_facts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    items = json.loads((resources / 'items.json').read_text())
    recipes = Recipes(resources)
    work = lab.LOCAL / 'crafter-parity'
    work.mkdir(parents=True, exist_ok=True)
    all_cases = list(cases(facts, items, recipes, args.count, args.seed))
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
            grid = sorted({i[4].split('"')[1] for i in case['items'] if i[:3] == [0, 0, 0]})
            front = [facts.describe(c[3]).split('[')[0] for c in case['cells'][1:2]] or ['air']
            groups.setdefault(f'{front[0]} <- {",".join(grid) or "empty"}', []).append((case, a, b))
    for kind, entries in sorted(errors.items(), key=lambda e: -len(e[1])):
        print(f'oracle error x{len(entries)}: {kind}')
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case ' + d.describe(case, facts) + ' disabled=' + json.dumps(case['disabled']) + f' ticks={case["ticks"]}')
            print('   native ' + ' | '.join(sorted(a)))
            print('   rust   ' + ' | '.join(sorted(b)))
    total = sum(len(v) for v in groups.values())
    skipped = sum(len(v) for v in errors.values())
    print(f'{len(all_cases) - total - skipped}/{len(all_cases) - skipped} cases match ({skipped} oracle errors)')
    h.finish(total + skipped)


if __name__ == '__main__':
    main()
