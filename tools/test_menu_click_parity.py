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
entity with slot state packets), the loom (with its pattern buttons), the
stonecutter (with its recipe buttons), the grindstone (enchanted, damaged and
cursed tools and books), the anvil (repairs, enchantment merges and renames,
with the player's levels), the smithing table (upgrades and trims), the
enchanting table (its button checks against the costs, lapis and levels), the
beacon (its payment slot and ServerboundSetBeaconPacket against the pyramid's
levels), the
cartography table (over a level with locked, scaled and missing maps: its
results, and the scaled and locked copies a take makes), the lectern (over the
block entity: its page buttons, clamped to the book's pages, and Take Book,
with the inventory compared too) and the
server's furnace menus over their block entities (`*_be`: a new input restarts
cooking, other clicks keep its progress). Bundles (empty, partly and fully
loaded, nested) are among the stacks and often in hand, for BundleItem's click
overrides. After every click the slots, carried stack, data slots, drops and
the player's sounds are compared.
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
         'crafter_be': 46, 'loom': 40, 'stonecutter': 38, 'grindstone': 39, 'anvil': 39, 'smithing': 40, 'enchanting': 38, 'beacon': 37, 'cartography': 39, 'lectern': 1, 'furnace_be': 39, 'blast_furnace_be': 39, 'smoker_be': 39}
# Where a carried bundle is common: the containers that hold anything.
BUNDLE_MENUS = {'inventory', 'chest1', 'chest3', 'shulker', 'hopper', 'dispenser'}
DATA = {'furnace': 4, 'blast_furnace': 4, 'smoker': 4, 'brewing': 4, 'crafter': 10, 'crafter_be': 10, 'loom': 1,
        'stonecutter': 1, 'enchanting': 10, 'beacon': 3, 'lectern': 1,
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
# Stonecutter inputs (several recipes each), and a block it doesn't cut.
CUTTABLE = ['stone', 'cobblestone', 'andesite', 'granite', 'copper_block', 'sandstone', 'quartz_block', 'deepslate',
            'stone_bricks', 'oak_planks']
# Grindstone inputs: tools and armor, books, and their enchantments (curses too).
GRINDABLE = ['iron_sword', 'diamond_pickaxe', 'bow', 'iron_helmet', 'elytra', 'shield', 'fishing_rod', 'trident']
# Beacon powers (BEACON_EFFECTS by level, and one no level gives) and payments.
POWERS = ['speed', 'haste', 'resistance', 'jump_boost', 'strength', 'regeneration', 'night_vision']
PAYMENTS = ['iron_ingot', 'gold_ingot', 'emerald', 'diamond', 'netherite_ingot']
ENCHANTS = ['sharpness', 'unbreaking', 'mending', 'efficiency', 'protection', 'power', 'looting', 'fortune',
            'binding_curse', 'vanishing_curse']


# Anvil renames (ServerboundRenameItemPacket): blank, too long, a section sign, an item's own name.
NAMES = ['', ' ', 'Blade', 'Iron Sword', 'Diamond Pickaxe', 'Enchanted Book', '\u00a7cRed', 'x' * 50, 'x' * 51]
# Smithing inputs: templates, bases and additions (trim materials and netherite).
TEMPLATES = ['netherite_upgrade_smithing_template', 'coast_armor_trim_smithing_template',
             'sentry_armor_trim_smithing_template', 'bolt_armor_trim_smithing_template']
BASES = ['diamond_sword', 'diamond_chestplate', 'diamond_pickaxe', 'iron_helmet', 'iron_chestplate', 'netherite_boots',
         'leather_leggings', 'golden_helmet', 'turtle_helmet', 'diamond_hoe']
ADDITIONS = ['netherite_ingot', 'iron_ingot', 'gold_ingot', 'emerald', 'redstone', 'quartz', 'amethyst_shard',
             'lapis_lazuli', 'copper_ingot', 'diamond']


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
    def __init__(self, rng, items, bundles=None):
        self.rng = rng
        # Bundles come from their own generator, so the other stacks stay as they were.
        self.bundles = bundles or random.Random(0)
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
        stack = snbt(item, max(1, count), damage, curse=rng.random() < 0.15 and 'equippable' in json.dumps(info), potion=potion)
        return self.bundle() if self.bundles.random() < 0.06 else stack

    def bundle(self, room=64):
        """A bundle, empty or holding up to its weight (in 64ths: 1 per item of a
        64-stack, 4 of a 16-stack, 64 of a tool), now and then another bundle."""
        return self.loaded_bundle(room)[0]

    def loaded_bundle(self, room):
        rng = self.bundles
        color = rng.choice(['', '', 'red_', 'light_blue_'])
        inside, used = [], 0
        for _ in range(rng.choice([0, 0, 1, 1, 2, 3, 5])):
            if room == 64 and rng.random() < 0.1:
                # A nested bundle weighs its contents plus 1/16.
                if room - used > 4:
                    nested, weight = self.loaded_bundle(min(room - used - 4, 16))
                    inside.append(nested)
                    used += weight + 4
                continue
            item = rng.choice(self.pool)
            top = self.items[item].get('minecraft:max_stack_size', 64)
            unit = 64 // top
            if unit > room - used or 'shulker_box' in item:
                continue
            most = min(top, (room - used) // unit)
            count = min(most, rng.choice([1, 1, 2, rng.randrange(1, most + 1)]))
            used += count * unit
            inside.append(snbt(item, count))
        body = ',components:{"minecraft:bundle_contents":[%s]}' % ','.join(inside) if inside else ''
        return '{id:"minecraft:%sbundle",count:1%s}' % (color, body), used

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

    def cut_stack(self):
        """Mostly something the stonecutter cuts, sometimes anything."""
        rng = self.rng
        if rng.random() < 0.75:
            return snbt('minecraft:' + rng.choice(CUTTABLE), rng.choice([1, 1, 2, 3, 10, 63, 64]))
        return self.stack()

    def grind_stack(self):
        """Mostly an enchanted or damaged tool or an enchanted book, sometimes anything."""
        rng = self.rng
        roll = rng.random()
        if roll > 0.8:
            return self.stack()
        levels = ','.join('"minecraft:%s":%d' % (e, rng.randrange(1, 4))
                          for e in rng.sample(ENCHANTS, rng.choice([0, 1, 1, 2, 3])))
        comps = []
        if roll < 0.25:
            item = 'minecraft:' + rng.choice(['enchanted_book', 'enchanted_book', 'book'])
            if levels:
                comps.append('"minecraft:stored_enchantments":{%s}' % levels)
            count = 1 if rng.random() < 0.85 else 2
        else:
            item = 'minecraft:' + rng.choice(GRINDABLE)
            info = self.items.get(item, {})
            if levels:
                comps.append('"minecraft:enchantments":{%s}' % levels)
            if info.get('minecraft:max_damage') and rng.random() < 0.6:
                comps.append('"minecraft:damage":%d' % rng.randrange(0, info['minecraft:max_damage']))
            count = 1
        if rng.random() < 0.2:
            comps.append('"minecraft:repair_cost":%d' % rng.choice([1, 3, 7]))
        body = ',components:{%s}' % ','.join(comps) if comps else ''
        return '{id:%s,count:%d%s}' % (quote(item), count, body)

    def anvil_stack(self):
        """A tool to repair or enchant, an enchanted book, a repair material, a named item or anything."""
        rng = self.rng
        roll = rng.random()
        if roll < 0.45:
            stack = self.grind_stack()
        elif roll < 0.65:
            item = rng.choice(['iron_ingot', 'diamond', 'leather', 'oak_planks', 'phantom_membrane', 'string'])
            stack = snbt('minecraft:' + item, rng.choice([1, 1, 2, 3, 5, 64]))
        else:
            stack = self.stack()
        if rng.random() < 0.15:
            name = '"minecraft:custom_name":"%s"' % rng.choice(['Blade', 'Iron Sword', 'Old'])
            stack = stack.replace(',components:{', ',components:{%s,' % name) if ',components:{' in stack \
                else stack[:-1] + ',components:{%s}}' % name
        return stack

    def smith_stack(self, slot=None):
        """A template, a base or an addition (for `slot`, else any), sometimes anything."""
        rng = self.rng
        if rng.random() < 0.15:
            return self.stack()
        slot = rng.randrange(3) if slot is None else slot
        pool = [TEMPLATES, BASES, ADDITIONS][slot]
        item = 'minecraft:' + rng.choice(pool)
        count = 1 if slot == 1 else rng.choice([1, 1, 2, 5, 64])
        if slot == 1 and rng.random() < 0.3:
            return '{id:%s,count:1,components:{"minecraft:enchantments":{"minecraft:unbreaking":2}}}' % quote(item)
        return snbt(item, count)

    def carto_stack(self):
        """A filled map (some ids without data), paper, an empty map, a glass pane or anything."""
        rng = self.rng
        roll = rng.random()
        if roll < 0.4:
            return '{id:"minecraft:filled_map",count:%d,components:{"minecraft:map_id":%d}}' % (
                rng.choice([1, 1, 2, 3, 64]), rng.randrange(0, 7))
        if roll < 0.75:
            return snbt('minecraft:' + rng.choice(['paper', 'map', 'glass_pane']), rng.choice([1, 1, 2, 5, 64]))
        if roll < 0.78:
            # Something else with a map id: the map slot takes it.
            return '{id:"minecraft:paper",count:1,components:{"minecraft:map_id":%d}}' % rng.randrange(0, 7)
        return self.stack()

    def book(self):
        """A written or writable book with a few pages, or something else."""
        rng = self.rng
        pages = ','.join('"p%d"' % i for i in range(rng.choice([0, 1, 2, 3, 5])))
        roll = rng.random()
        if roll < 0.45:
            return ('{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":'
                    '{title:"T",author:"A",pages:[%s]}}}' % pages)
        if roll < 0.85:
            return '{id:"minecraft:writable_book",count:1,components:{"minecraft:writable_book_content":{pages:[%s]}}}' % pages
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
            if menu == 'stonecutter' and rng.random() < 0.25:
                # A recipe button (clickMenuButton), sometimes out of range or the selected one.
                out.append([rng.choice([0, 0, 1, 2, 3, 5, 10, 30, -1]), 0, 'BUTTON'])
                continue
            if menu == 'stonecutter' and rng.random() < 0.2:
                out.append([1, rng.randrange(0, 2), rng.choice(['PICKUP', 'QUICK_MOVE', 'QUICK_MOVE', 'THROW', 'SWAP'])])
                continue
            if menu == 'anvil' and rng.random() < 0.15:
                out.append([0, 0, 'RENAME', rng.choice(NAMES)])
                continue
            if menu in ('anvil', 'smithing') and rng.random() < 0.2:
                result = 2 if menu == 'anvil' else 3
                out.append([result, rng.randrange(0, 2), rng.choice(['PICKUP', 'QUICK_MOVE', 'QUICK_MOVE', 'THROW', 'SWAP'])])
                continue
            if menu == 'enchanting' and rng.random() < 0.25:
                # An offer button (clickMenuButton), sometimes out of range.
                out.append([rng.choice([0, 1, 2, 2, 3, -1]), 0, 'BUTTON'])
                continue
            if menu == 'beacon' and rng.random() < 0.3:
                # ServerboundSetBeaconPacket: powers, valid or not, or none.
                pick = lambda: 'minecraft:' + rng.choice(POWERS) if rng.random() < 0.8 else None
                primary, secondary = pick(), pick()
                if primary is None and secondary not in (None, 'minecraft:regeneration', 'minecraft:night_vision'):
                    # validateEffects compares a null primary (Holder.equals: a NullPointerException).
                    primary = secondary
                out.append([0, 0, 'SET_BEACON', primary, secondary])
                continue
            if menu == 'grindstone' and rng.random() < 0.2:
                out.append([2, rng.randrange(0, 2), rng.choice(['PICKUP', 'QUICK_MOVE', 'QUICK_MOVE', 'THROW', 'SWAP'])])
                continue
            if menu == 'cartography' and rng.random() < 0.3:
                # The result: take it, shift it out, throw it, swap it, clone it.
                out.append([2, rng.randrange(0, 2), rng.choice(['PICKUP', 'QUICK_MOVE', 'QUICK_MOVE', 'THROW', 'SWAP', 'CLONE'])])
                continue
            if menu == 'lectern':
                # The page buttons, a page jump, Take Book, and buttons it doesn't have; now and
                # then a click on the book's slot. Not SWAP or the drags: vanilla's bookAccess
                # hands out the block entity's own stack, which the swapped slot then shares.
                if rng.random() < 0.7:
                    out.append([rng.choice([1, 1, 2, 2, 2, 3, 100, 101, 102, 104, 0, 4, 99, -1]), 0, 'BUTTON'])
                else:
                    out.append([rng.choice([0, 0, -999]), rng.randrange(0, 2),
                                rng.choice(['PICKUP', 'THROW', 'QUICK_MOVE', 'CLONE'])])
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
    g = Gen(rng, items, random.Random(args.seed + 1))
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
        if menu == 'stonecutter':
            for i in rng.sample(range(2, size), 8):
                slots[i] = g.cut_stack()
            slots[0] = g.cut_stack() if rng.random() < 0.8 else '-'
            # The result follows the input and the selected recipe; a fixture stack there is rare.
            slots[1] = '-' if rng.random() < 0.9 else g.cut_stack()
        if menu == 'grindstone':
            for i in rng.sample(range(3, size), 10):
                slots[i] = g.grind_stack()
            slots[0] = g.grind_stack() if rng.random() < 0.7 else '-'
            slots[1] = g.grind_stack() if rng.random() < 0.6 else '-'
            # Two of the same tool, so mergeItems runs.
            if slots[0] != '-' and rng.random() < 0.3:
                slots[1] = slots[0] if rng.random() < 0.4 else g.grind_stack()
            # The result follows the inputs; a fixture stack there is rare.
            slots[2] = '-' if rng.random() < 0.9 else g.grind_stack()
        if menu == 'anvil':
            for i in rng.sample(range(3, size), 10):
                slots[i] = g.anvil_stack()
            slots[0] = g.anvil_stack() if rng.random() < 0.8 else '-'
            slots[1] = g.anvil_stack() if rng.random() < 0.6 else '-'
            if slots[0] != '-' and rng.random() < 0.25:
                slots[1] = slots[0]
            slots[2] = '-' if rng.random() < 0.9 else g.anvil_stack()
        if menu == 'smithing':
            for i in rng.sample(range(4, size), 12):
                slots[i] = g.smith_stack()
            for i in range(3):
                slots[i] = g.smith_stack(i) if rng.random() < 0.75 else '-'
            slots[3] = '-' if rng.random() < 0.9 else g.smith_stack()
        if menu == 'enchanting':
            lapis = lambda: snbt('minecraft:lapis_lazuli', rng.choice([1, 2, 3, 5, 64]))
            enchantable = lambda: snbt('minecraft:' + rng.choice(['iron_sword', 'book', 'diamond_pickaxe', 'bow',
                                                                  'golden_helmet', 'fishing_rod']), 1)
            for i in rng.sample(range(2, size), 10):
                slots[i] = rng.choice([lapis, enchantable, g.grind_stack])()
            slots[0] = rng.choice([enchantable, enchantable, g.grind_stack, g.stack])() if rng.random() < 0.8 else '-'
            slots[1] = lapis() if rng.random() < 0.7 else rng.choice(['-', g.stack()])
        if menu == 'beacon':
            pay = lambda: snbt('minecraft:' + rng.choice(PAYMENTS), rng.choice([1, 1, 1, 2, 64]))
            for i in rng.sample(range(1, size), 8):
                slots[i] = pay()
            slots[0] = pay() if rng.random() < 0.7 else rng.choice(['-', g.stack()])
        if menu == 'cartography':
            for i in rng.sample(range(3, size), 12):
                slots[i] = g.carto_stack()
            slots[0] = g.carto_stack() if rng.random() < 0.8 else '-'
            slots[1] = g.carto_stack() if rng.random() < 0.75 else '-'
            # The result follows the inputs; a fixture stack there is rare.
            slots[2] = '-' if rng.random() < 0.9 else g.carto_stack()
        if menu == 'lectern':
            slots[0] = g.book() if rng.random() < 0.85 else '-'
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
            if menu in ('loom', 'stonecutter'):
                return rng.choice([-1, -1, 0, 0, 1, 2, 3, 30])
            if menu == 'lectern':
                return rng.choice([-1, 0, 0, 1, 2, 4, 9])
            if menu == 'beacon':
                # the levels, then the powers (encodeEffect: id + 1)
                return rng.choice([0, 1, 2, 3, 4, 4]) if i == 0 else rng.choice([0, 0, 2, 4, 11])
            if menu == 'enchanting':
                # costs, the seed, then the clues
                return [rng.choice([0, 1, 2, 5, 15, 30]) for _ in range(3)][0] if i < 3 else \
                    rng.randrange(-2**15, 2**15) if i == 3 else rng.choice([-1, -1, 0, 3, 12])
            return 0
        carried = {'loom': g.loom_stack, 'stonecutter': g.cut_stack, 'grindstone': g.grind_stack,
                   'anvil': g.anvil_stack, 'smithing': g.smith_stack}.get(menu, g.stack)() if rng.random() < 0.5 else '-'
        if carried_food is not None:
            carried = carried_food
        elif menu in BUNDLE_MENUS and g.bundles.random() < 0.3:
            # A bundle in hand, emptied into slots and filled from them.
            carried = g.bundle()
        if menu == 'lectern':
            carried = '-'
        extra = {}
        if menu == 'cartography':
            # Maps 0-4 exist (5 and 6 don't): [id, scale, locked].
            extra['maps'] = [[i, rng.choice([0, 0, 1, 3, 4]), rng.random() < 0.25] for i in range(5)]
            if rng.random() < 0.5:
                carried = g.carto_stack()
        if menu == 'lectern':
            # The inventory Take Book adds to (full, so the book drops, now and then).
            density = rng.choice([0.0, 0.3, 1.0])
            extra['inventory'] = [g.stack() if rng.random() < density else '-' for _ in range(36)]
        cases.append({'menu': menu, 'creative': rng.random() < 0.25, 'slots': slots, **extra,
                      'levels': rng.choice([0, 1, 3, 8, 30, 39, 40]),
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
