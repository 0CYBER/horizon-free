#!/usr/bin/env python3
"""Compare AbstractContainerMenu.clicked with vanilla's menu classes.

The Java oracle (VanillaMenuOracle) runs each case's clicks on the real menu
class with a stand-in player and level; the Rust example (menu_fixture) runs
horizon_client::menu. Cases are randomized slot contents with random click
sequences covering every ContainerInput (pickup, quick move, swap, clone,
throw, the three quick craft drags, pickup all, and the screens' Shift double
click: QUICK_MOVE on every slot of the clicked slot's container holding the
item the first click moved) over the player inventory,
crafting table, chests, hopper, dispenser, shulker box, the furnaces, the
brewing stand, the crafter (the client's menu, and the server's over the block
entity with slot state packets), the loom (with its pattern buttons) and the
server's furnace menus over their block entities (`*_be`: a new input restarts
cooking, other clicks keep its progress). After
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
         'chest6': 90, 'hopper': 41, 'dispenser': 45, 'shulker': 63, 'furnace': 39, 'blast_furnace': 39, 'smoker': 39, 'brewing': 41, 'crafter': 46,
         'crafter_be': 46, 'loom': 40, 'furnace_be': 39, 'blast_furnace_be': 39, 'smoker_be': 39}
DATA = {'furnace': 4, 'blast_furnace': 4, 'smoker': 4, 'brewing': 4, 'crafter': 10, 'crafter_be': 10, 'loom': 1,
        'furnace_be': 4, 'blast_furnace_be': 4, 'smoker_be': 4}
DYES = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan', 'purple',
        'blue', 'brown', 'green', 'red', 'black']
PATTERNS = ['minecraft:' + p for p in ('base', 'square_bottom_left', 'stripe_bottom', 'cross', 'border', 'gradient',
                                      'creeper', 'flower', 'globe', 'rhombus', 'small_stripes', 'half_horizontal')]
COMMON = ['stone', 'dirt', 'cobblestone', 'oak_planks', 'stick', 'coal', 'iron_ingot', 'diamond', 'apple', 'bread',
          'oak_log', 'sand', 'bucket', 'water_bucket', 'lava_bucket', 'ender_pearl', 'snowball', 'egg', 'white_wool',
          'iron_helmet', 'diamond_chestplate', 'leather_leggings', 'iron_boots', 'shield', 'carved_pumpkin',
          'elytra', 'turtle_helmet', 'iron_sword', 'diamond_pickaxe', 'bow', 'shulker_box', 'red_shulker_box',
          'raw_iron', 'potato', 'beef', 'chicken', 'sticky_piston', 'blaze_rod', 'lava_bucket', 'oak_sapling',
          'white_dye', 'red_dye', 'torch', 'crafting_table', 'furnace', 'chest', 'glass_bottle', 'nether_wart', 'blaze_powder',
          'redstone', 'gunpowder', 'sugar', 'potion', 'splash_potion', 'lingering_potion', 'fermented_spider_eye', 'dragon_breath']


def quote(text):
    return '"' + text + '"'


def snbt(item, count, damage=0, curse=False, potion=None):
    comps = []
    if potion:
        comps.append('"minecraft:potion_contents":{potion:"%s"}' % potion)
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
        potion = rng.choice(['minecraft:water', 'minecraft:awkward', 'minecraft:healing', 'minecraft:thick']) if item.endswith('potion') and rng.random() < 0.8 else None
        return snbt(item, max(1, count), damage, curse=rng.random() < 0.15 and 'equippable' in json.dumps(info), potion=potion)

    def slots(self, size, density):
        return [self.stack() if self.rng.random() < density else '-' for _ in range(size)]

    def loom_stack(self, roll=None):
        """A banner (with up to seven layers), a dye, a pattern item or anything."""
        rng = self.rng
        roll = rng.random() if roll is None else roll
        if roll < 0.3:
            layers = ','.join('{pattern:"%s",color:"%s"}' % (rng.choice(PATTERNS), rng.choice(DYES))
                              for _ in range(rng.choice([0, 0, 1, 2, 5, 6, 6, 7])))
            extra = ',components:{"minecraft:banner_patterns":[%s]}' % layers if layers else ''
            return '{id:"minecraft:%s_banner",count:%d%s}' % (rng.choice(DYES), rng.choice([1, 1, 2, 16]), extra)
        if roll < 0.55:
            return snbt('minecraft:%s_dye' % rng.choice(DYES), rng.choice([1, 1, 2, 3, 64]))
        if roll < 0.75:
            pool = [k for k in self.items if k.endswith('_banner_pattern')]
            return snbt(rng.choice(pool), 1)
        return self.stack()

    def recipe(self, slots, width, start=1):
        """Fills the crafting grid (slots `start`..) with a small known recipe."""
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
                slots[start + x + y * width] = snbt('minecraft:' + name, amount() if name != 'milk_bucket' else 1)

    def ops(self, size, menu, slots):
        rng = self.rng
        out = []
        for _ in range(rng.randrange(1, 9)):
            if menu == 'loom' and rng.random() < 0.25:
                # A pattern button (clickMenuButton), sometimes out of range.
                out.append([rng.choice([0, 0, 1, 2, 3, 5, 10, 20, 39, 40, -1]), 0, 'BUTTON'])
                continue
            if menu == 'loom' and rng.random() < 0.15:
                # The result: take it, shift it out, throw it.
                out.append([3, rng.randrange(0, 2), rng.choice(['PICKUP', 'QUICK_MOVE', 'THROW', 'SWAP'])])
                continue
            if menu == 'crafter_be' and rng.random() < 0.2:
                out.append([rng.choice([0, 1, 4, 8, 8, 9, -1]), rng.randrange(0, 2), 'SLOT_STATE'])
                continue
            kind = rng.choices(['PICKUP', 'QUICK_MOVE', 'SWAP', 'CLONE', 'THROW', 'DRAG', 'PICKUP_ALL', 'JUNK',
                                'QUICK_MOVE_ALL'], [30, 20, 8, 3, 6, 14, 6, 2, 0 if menu.endswith('_be') else 6])[0]
            slot = 0 if rng.random() < 0.2 and size in (46,) else rng.randrange(size)
            if kind == 'QUICK_MOVE_ALL':
                # A Shift double click: lastQuickMoved is mostly what a slot of the menu holds.
                held = [s for s in slots if s != '-']
                moved = rng.choice(held) if held and rng.random() < 0.85 else self.stack()
                out.append([slot, 0, kind, moved])
                continue
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
    parser.add_argument('--cases', type=int, default=8000)
    parser.add_argument('--limit', type=int, default=3)
    parser.add_argument('--menus', help='comma-separated menus to run (default: all)')
    args = parser.parse_args()
    work = lib.work_dir('menu-click-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaMenuOracle')
    items = json.loads((lib.RESOURCES / 'items.json').read_text())
    rng = random.Random(args.seed)
    g = Gen(rng, items)
    cases = []
    chosen = args.menus.split(',') if args.menus else list(MENUS)
    for n in range(args.cases):
        menu = chosen[n % len(chosen)]
        size = MENUS[menu]
        slots = g.slots(size, rng.choice([0.1, 0.4, 0.8]))
        if menu == 'loom':
            for i in rng.sample(range(4, size), 8):
                slots[i] = g.loom_stack()
            # Mostly a banner, a dye and sometimes a pattern item in their slots.
            slots[0] = g.loom_stack(0.0) if rng.random() < 0.75 else rng.choice(['-', g.loom_stack()])
            slots[1] = g.loom_stack(0.4) if rng.random() < 0.75 else rng.choice(['-', g.loom_stack()])
            slots[2] = g.loom_stack(0.6) if rng.random() < 0.4 else rng.choice(['-', '-', g.loom_stack()])
            # The result slot follows the inputs; a fixture stack there is rare.
            slots[3] = '-' if rng.random() < 0.9 else g.loom_stack()
        if menu == 'crafter_be':
            slots[45] = '-'
            if rng.random() < 0.6:
                g.recipe(slots, 3, start=0)
        if menu in ('inventory', 'crafting'):
            slots[0] = '-'
        if menu in ('inventory', 'crafting') and rng.random() < 0.6:
            g.recipe(slots, 2 if menu == 'inventory' else 3)
        if menu.endswith('_be') and menu != 'crafter_be' and rng.random() < 0.7:
            # Mostly something cooking: a smeltable input, the same in the
            # inventory and in hand, so merges and partial takes happen.
            food = snbt('minecraft:' + rng.choice(['raw_iron', 'beef', 'potato', 'sand', 'cobblestone', 'oak_log']),
                        rng.choice([1, 2, 5, 32, 64]))
            slots[0] = food
            for i in rng.sample(range(3, size), 4):
                slots[i] = food if rng.random() < 0.6 else g.stack()
            carried_food = food if rng.random() < 0.5 else None
        else:
            carried_food = None
        def datum(i):
            if menu.endswith('_be') and menu != 'crafter_be':
                # litTimeRemaining, litTotalTime, cookingTimer, cookingTotalTime
                return [rng.choice([0, 37, 1600]), 1600, rng.choice([0, 1, 57, 199]), rng.choice([200, 100])][i]
            if menu in ('crafter', 'crafter_be'):
                return rng.choice([0, 0, 1])
            if menu == 'loom':
                return rng.choice([-1, -1, 0, 0, 1, 2, 3, 30])
            return 0
        carried = (g.loom_stack() if menu == 'loom' else g.stack()) if rng.random() < 0.5 else '-'
        if carried_food is not None:
            carried = carried_food
        cases.append({'menu': menu, 'creative': rng.random() < 0.25, 'slots': slots,
                      'carried': carried, 'data': [datum(i) for i in range(DATA.get(menu, 0))],
                      'ops': g.ops(size, menu, slots)})
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
