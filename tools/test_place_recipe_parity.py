#!/usr/bin/env python3
"""Compare the server side of recipe book clicks with vanilla's ServerPlaceRecipe.

Each case opens the inventory, a crafting table or a furnace (smelting,
blasting, smoking) on a player whose inventory holds the items of a random
recipe of the book (shaped, shapeless, transmuting, dyeing, imbuing and
cooking recipes), with more or fewer than needed, damaged, enchanted and
renamed copies the book must not use, junk, and sometimes a nearly full
inventory; the grid is empty, holds the same recipe already (placing again
adds one set, or a full stack with shift) or something else. The Java oracle
(VanillaPlaceRecipeOracle) runs RecipeBookMenu.handlePlacement as the server's
packet handler does; examples/probe/place_recipe_fixture runs
horizon_client::menu's place_recipe. Every menu slot, the whole inventory,
the drops and whether the client gets a ghost recipe are compared.

Vanilla counts the available items in a hash map ordered by identity hash
codes, an order that changes between runs and decides which of several
fitting items fills an ingredient; the oracle writes the order its run used
and the Rust side follows it.
"""
import argparse
import json
import random
try:
    from . import interaction_lab as lib, test_crafter_parity as crafter
except ImportError:
    import interaction_lab as lib
    import test_crafter_parity as crafter

GRIDS = {'inventory': 2, 'crafting': 3}
COOKING = {'furnace': 'minecraft:smelting', 'blast_furnace': 'minecraft:blasting', 'smoker': 'minecraft:smoking'}
CRAFTING_TYPES = ('minecraft:crafting_shaped', 'minecraft:crafting_shapeless', 'minecraft:crafting_transmute',
                  'minecraft:crafting_dye', 'minecraft:crafting_imbue')
JUNK = ['minecraft:dirt', 'minecraft:stone', 'minecraft:diamond', 'minecraft:stick', 'minecraft:oak_planks',
        'minecraft:apple', 'minecraft:feather', 'minecraft:coal', 'minecraft:iron_ingot', 'minecraft:cobblestone']


def stack(item, count, kind=None, rng=None):
    comps = []
    if kind == 'damaged':
        comps.append('"minecraft:damage":%d' % rng.randrange(1, 30))
    elif kind == 'enchanted':
        comps.append('"minecraft:enchantments":{"minecraft:unbreaking":1}')
    elif kind == 'named':
        comps.append('"minecraft:custom_name":"x"')
    body = ',components:{%s}' % ','.join(comps) if comps else ''
    return '{id:"%s",count:%d%s}' % (item, count, body)


class Gen:
    def __init__(self, rng, items, recipes):
        self.rng = rng
        self.items = items
        self.recipes = recipes
        self.by_type = {}
        for name, r in recipes.recipes.items():
            self.by_type.setdefault(r['type'], []).append(name)

    def top(self, item):
        return self.items.get(item, {}).get('minecraft:max_stack_size', 64)

    def ingredients(self, r):
        """Each ingredient's item choices, as PlacementInfo lists them."""
        t = r['type']
        if t == 'minecraft:crafting_shaped':
            return [self.recipes.choices(r['key'][ch]) for row in r['pattern'] for ch in row if ch != ' ']
        if t == 'minecraft:crafting_shapeless':
            return [self.recipes.choices(i) for i in r['ingredients']]
        if t == 'minecraft:crafting_transmute':
            n = r.get('material_count', {})
            top = n.get('max', 1) if isinstance(n, dict) else n
            return [self.recipes.choices(r['input'])] + [self.recipes.choices(r['material'])] * top
        if t == 'minecraft:crafting_dye':
            return [self.recipes.choices(r['target']), self.recipes.choices(r['dye'])]
        if t == 'minecraft:crafting_imbue':
            return [self.recipes.choices(r['material'])] * 8 + [self.recipes.choices(r['source'])]
        return [self.recipes.choices(r['ingredient'])]

    def fits(self, r, size):
        t = r['type']
        if t == 'minecraft:crafting_shaped':
            return len(r['pattern']) <= size and max(len(row) for row in r['pattern']) <= size
        return len(self.ingredients(r)) <= size * size

    def case(self, menu):
        rng = self.rng
        if menu in COOKING:
            name = rng.choice(self.by_type.get(COOKING[menu], []))
        else:
            size = GRIDS[menu]
            pool = [n for t in CRAFTING_TYPES for n in self.by_type.get(t, [])]
            for _ in range(50):
                name = rng.choice(pool)
                if self.fits(self.recipes.recipes[name], size):
                    break
        r = self.recipes.recipes[name]
        ingredients = self.ingredients(r)
        inventory = {}
        free = list(range(36))
        rng.shuffle(free)

        def put(item_stack):
            if free:
                inventory[free.pop()] = item_stack
        for choices in ingredients:
            if not choices or rng.random() < 0.08:
                continue
            for item in rng.sample(choices, min(len(choices), rng.choice([1, 1, 1, 2, 3]))):
                top = self.top(item)
                for _ in range(rng.choice([1, 1, 1, 2, 3])):
                    count = 1 if top == 1 else rng.choice([1, 1, 2, 3, 7, top, top - 1, rng.randrange(1, top + 1)])
                    kind = rng.choice([None] * 12 + ['damaged', 'enchanted', 'named'])
                    if kind == 'damaged' and not self.items.get(item, {}).get('minecraft:max_damage'):
                        kind = None
                    put(stack(item, max(1, count), kind, rng))
        fill = rng.choice([0, 0, 3, 8, 30, 36])
        for _ in range(fill):
            item = rng.choice(JUNK)
            put(stack(item, rng.choice([1, 16, 64, self.top(item)])))
        if rng.random() < 0.15:
            inventory[40] = stack(rng.choice(JUNK + [c[0] for c in ingredients if c]), rng.choice([1, 5, 64]))
        grid = []
        roll = rng.random()
        if menu in COOKING:
            if roll < 0.3:
                grid.append([0, stack(rng.choice(ingredients[0]), rng.choice([1, 3, 64]))])
            elif roll < 0.5:
                grid.append([0, stack(rng.choice(JUNK), rng.choice([1, 64]))])
            if rng.random() < 0.4:
                grid.append([1, stack('minecraft:coal', rng.choice([1, 64]))])
            if rng.random() < 0.4:
                result = r['result']['id'] if isinstance(r['result'], dict) else r['result']
                grid.append([2, stack(rng.choice([result, 'minecraft:stone']), rng.choice([1, 5, 64]))])
        else:
            size = GRIDS[menu]
            if roll < 0.35:
                # The same recipe already in the grid (a placed or crafted set).
                cells = self.recipes.grid(rng, name) if size == 3 else None
                if cells is None:
                    cells = [None] * 9
                    for i, choices in enumerate(ingredients[:size * size]):
                        if choices:
                            cells[(i // size) * 3 + i % size] = rng.choice(choices).split(':')[1]
                for i, cell in enumerate(cells):
                    x, y = i % 3, i // 3
                    if cell and x < size and y < size:
                        item = 'minecraft:' + cell
                        grid.append([1 + x + y * size, stack(item, rng.choice([1, 1, 2, 5, self.top(item)]))])
            elif roll < 0.6:
                for i in range(size * size):
                    if rng.random() < 0.4:
                        grid.append([1 + i, stack(rng.choice(JUNK), rng.choice([1, 3, 64]))])
        return {'menu': menu, 'recipe': 'minecraft:' + name, 'max': rng.random() < 0.5, 'creative': rng.random() < 0.2,
                'selected': rng.randrange(9), 'inventory': sorted([k, v] for k, v in inventory.items()), 'grid': grid}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=11)
    parser.add_argument('--cases', type=int, default=6000)
    parser.add_argument('--limit', type=int, default=3)
    args = parser.parse_args()
    work = lib.work_dir('place-recipe-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaMenuOracle', 'VanillaPlaceRecipeOracle')
    items = json.loads((lib.RESOURCES / 'items.json').read_text())
    gen = Gen(random.Random(args.seed), items, crafter.Recipes(lib.RESOURCES))
    menus = ['inventory', 'crafting', 'crafting', 'furnace', 'blast_furnace', 'smoker']
    cases = [gen.case(menus[n % len(menus)]) for n in range(args.cases)]
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in cases))
    vanilla.run('VanillaPlaceRecipeOracle', work / 'cases.jsonl', work / 'native.txt', work / 'orders.txt')
    lib.cargo_example('horizon-client', 'place_recipe_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'orders.txt',
                      work / 'rust.txt')
    bad = lib.compare(cases, work / 'native.txt', work / 'rust.txt',
                      lambda case, line: f'{case["menu"]} {line.split(" ")[1] if line.startswith("post") else "error"}',
                      limit=args.limit)
    errors = sum(1 for line in (work / 'native.txt').read_text().splitlines() if line.startswith('error-case'))
    if errors:
        print(f'{errors} oracle errors')
    raise SystemExit(1 if bad or errors else 0)


if __name__ == '__main__':
    main()
