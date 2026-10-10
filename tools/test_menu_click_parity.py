#!/usr/bin/env python3
"""Compare AbstractContainerMenu.clicked with vanilla's menu classes.

The Java oracle (VanillaMenuOracle) runs each case's clicks on the real menu
class with a stand-in player and level; the Rust example (menu_fixture) runs
horizon_client::menu. Cases are randomized slot contents with random click
sequences covering every ContainerInput (pickup, quick move, swap, clone,
throw, the three quick craft drags, pickup all) over the player inventory,
crafting table, chests, hopper, dispenser, shulker box and the furnaces. After
every click the slots, carried stack, data slots and drops are compared.
"""
import argparse
import json
import random
try:
    from . import interaction_lab as lib, lab
except ImportError:
    import interaction_lab as lib
    import lab

MENUS = {'inventory': 46, 'crafting': 46, 'chest1': 45, 'chest2': 54, 'chest3': 63, 'chest4': 72, 'chest5': 81,
         'chest6': 90, 'hopper': 41, 'dispenser': 45, 'shulker': 63, 'furnace': 39, 'blast_furnace': 39, 'smoker': 39}
DATA = {'furnace': 4, 'blast_furnace': 4, 'smoker': 4}
COMMON = ['stone', 'dirt', 'cobblestone', 'oak_planks', 'stick', 'coal', 'iron_ingot', 'diamond', 'apple', 'bread',
          'oak_log', 'sand', 'bucket', 'water_bucket', 'lava_bucket', 'ender_pearl', 'snowball', 'egg', 'white_wool',
          'iron_helmet', 'diamond_chestplate', 'leather_leggings', 'iron_boots', 'shield', 'carved_pumpkin',
          'elytra', 'turtle_helmet', 'iron_sword', 'diamond_pickaxe', 'bow', 'shulker_box', 'red_shulker_box',
          'raw_iron', 'potato', 'beef', 'chicken', 'sticky_piston', 'blaze_rod', 'lava_bucket', 'oak_sapling',
          'white_dye', 'red_dye', 'torch', 'crafting_table', 'furnace', 'chest']


def quote(text):
    return '"' + text + '"'


def snbt(item, count, damage=0, curse=False):
    comps = []
    if damage:
        comps.append('"minecraft:damage":%d' % damage)
    if curse:
        comps.append('"minecraft:enchantments":{"minecraft:binding_curse":1}')
    body = ',components:{%s}' % ','.join(comps) if comps else ''
    return '{id:%s,count:%d%s}' % (quote(item), count, body)


class Gen:
    def __init__(self, rng, items):
        self.rng = rng
        self.items = items
        self.pool = ['minecraft:' + c for c in COMMON if 'minecraft:' + c in items]
        self.all = [k for k, v in items.items() if k != 'minecraft:air' and 'bundle' not in k
                    and not k.endswith('_spawn_egg')]

    def stack(self):
        rng = self.rng
        item = rng.choice(self.pool if rng.random() < 0.85 else self.all)
        info = self.items[item]
        top = info.get('minecraft:max_stack_size', 64)
        damage = 0
        if info.get('minecraft:max_damage') and rng.random() < 0.3:
            damage = rng.randrange(1, info['minecraft:max_damage'])
        count = 1 if top == 1 else rng.choice([1, 1, 2, top, top, top - 1, rng.randrange(1, top + 1)])
        return snbt(item, max(1, count), damage, curse=rng.random() < 0.15 and 'equippable' in json.dumps(info))

    def slots(self, size, density):
        return [self.stack() if self.rng.random() < density else '-' for _ in range(size)]

    def recipe(self, slots, width):
        """Fills the crafting grid (slots 1..) with a small known recipe."""
        rng = self.rng
        amount = lambda: rng.choice([1, 1, 2, 5, 64])
        choices = [[('oak_log',)], [('oak_planks',), ('oak_planks',)], [('oak_planks', 'oak_planks'), ('oak_planks', 'oak_planks')],
                   [('iron_ingot',) * 3] * 3, [('milk_bucket',) * 3, ('sugar', 'egg', 'sugar'), ('wheat',) * 3],
                   [('wheat',) * 3], [('white_wool',) * 2, ('white_wool',) * 2], [('coal',), ('stick',)]]
        pattern = rng.choice(choices)
        if len(pattern) > width or any(len(row) > width for row in pattern):
            pattern = [('oak_planks',), ('oak_planks',)]
        for y, row in enumerate(pattern):
            for x, name in enumerate(row):
                slots[1 + x + y * width] = snbt('minecraft:' + name, amount() if name != 'milk_bucket' else 1)

    def ops(self, size):
        rng = self.rng
        out = []
        for _ in range(rng.randrange(1, 9)):
            kind = rng.choices(['PICKUP', 'QUICK_MOVE', 'SWAP', 'CLONE', 'THROW', 'DRAG', 'PICKUP_ALL', 'JUNK'],
                               [30, 20, 8, 3, 6, 14, 6, 2])[0]
            slot = 0 if rng.random() < 0.2 and size in (46,) else rng.randrange(size)
            if kind == 'PICKUP':
                out.append([rng.choice([slot, slot, slot, -999, -1]), rng.randrange(0, 3), kind])
            elif kind == 'QUICK_MOVE':
                out.append([rng.choice([slot, slot, slot, -999, -1]), rng.randrange(0, 3), kind])
            elif kind == 'SWAP':
                out.append([slot, rng.choice([0, 1, 2, 3, 4, 5, 6, 7, 8, 40, 9, -1]), kind])
            elif kind in ('CLONE', 'THROW', 'PICKUP_ALL'):
                out.append([slot, rng.randrange(0, 2), kind])
            elif kind == 'DRAG':
                ty = rng.randrange(0, 4)
                out.append([-999, 0 | ty << 2, 'QUICK_CRAFT'])
                for _ in range(rng.randrange(0, 6)):
                    out.append([rng.randrange(size), 1 | ty << 2, 'QUICK_CRAFT'])
                out.append([-999, 2 | ty << 2, 'QUICK_CRAFT'])
            else:
                out.append([rng.choice([slot, 999, -5]), rng.randrange(-1, 5), rng.choice(
                    ['PICKUP', 'QUICK_MOVE', 'QUICK_CRAFT', 'SWAP', 'THROW', 'PICKUP_ALL'])])
        return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=7)
    parser.add_argument('--cases', type=int, default=6000)
    parser.add_argument('--limit', type=int, default=3)
    args = parser.parse_args()
    work = lib.work_dir('menu-click-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaMenuOracle')
    items = json.loads((lib.RESOURCES / 'items.json').read_text())
    rng = random.Random(args.seed)
    g = Gen(rng, items)
    cases = []
    for n in range(args.cases):
        menu = list(MENUS)[n % len(MENUS)]
        size = MENUS[menu]
        slots = g.slots(size, rng.choice([0.1, 0.4, 0.8]))
        if menu in ('inventory', 'crafting'):
            slots[0] = '-'
        if menu in ('inventory', 'crafting') and rng.random() < 0.6:
            g.recipe(slots, 2 if menu == 'inventory' else 3)
        cases.append({'menu': menu, 'creative': rng.random() < 0.25, 'slots': slots,
                      'carried': g.stack() if rng.random() < 0.5 else '-', 'data': [0] * DATA.get(menu, 0),
                      'ops': g.ops(size)})
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in cases))
    vanilla.run('VanillaMenuOracle', work / 'cases.jsonl', work / 'native.txt')
    lib.cargo_example('horizon-client', 'menu_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'rust.txt')
    native = (work / 'native.txt').read_text().split('\n')
    rust = (work / 'rust.txt').read_text().split('\n')

    def by_case(lines):
        out, cur = [], []
        for line in lines:
            cur.append(line)
            if line == 'end' or line.startswith('error-case'):
                out.append(cur)
                cur = []
        return out
    native, rust = by_case(native), by_case(rust)
    bad = {}
    for case, a, b in zip(cases, native, rust):
        if a != b:
            step = next(i for i, (x, y) in enumerate(zip(a, b)) if x != y) if len(a) == len(b) else 0
            op = case['ops'][step - 1] if 0 < step <= len(case['ops']) else None
            bad.setdefault((case['menu'], op[2] if op else 'init'), []).append((case, step, a, b))
    total = sum(len(v) for v in bad.values())
    for key, entries in sorted(bad.items(), key=lambda e: -len(e[1])):
        print(f'{key}: {len(entries)} differing')
        for case, step, a, b in entries[:args.limit]:
            print(f'  ops    {case["ops"][:step]}\n  start  {a[0]}\n  native {a[step]}\n  rust   {b[step]}')
    print(f'{len(cases) - total}/{len(cases)} cases match')
    raise SystemExit(1 if total else 0)


if __name__ == '__main__':
    main()
