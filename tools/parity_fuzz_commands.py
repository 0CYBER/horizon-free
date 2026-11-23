"""The `commands` family for tools/parity_fuzz.py: commands generated from the server's own command tree (#20446).

The prepared `commands.json` is vanilla's command tree: every literal, every argument with its parser and limits,
and which nodes run. A command here is a walk from a root to a node that runs, each argument filled by its parser's
generator: positions and entities are the scenario's own (a barrel with two stacks, a chest, a stone block, an armor
stand, a pig and an item entity), names come from the prepared registries, stacks and predicates from the `items`
family. A scenario runs a few such commands, compares each one's feedback and results, and then reads every fixture.

Left out: roots that change the world beyond the scenario's chunk or the player (time, weather, game rules, bans,
scores and teams, which every later scenario of the batch would see), command storage (the same), arguments that name players, and parsers
with no generator yet (the walk gives up there and starts again).
"""
import json
import os
from pathlib import Path

try:
    from . import lab, parity_fuzz_data as data
    from .parity_fuzz import stack_text
except ImportError:
    import lab
    import parity_fuzz_data as data
    from parity_fuzz import stack_text

TREE = {}  # commands.json
NAMES = {}  # command-registries.json: 'effects' -> ids, and registries.json's particle types and blocks

# Roots whose effects stay with the scenario's own blocks and entities.
ROOTS = ['attribute', 'clone', 'damage', 'data', 'effect', 'enchant', 'execute', 'fill', 'fillbiome', 'item', 'kill',
         'loot', 'particle', 'ride', 'rotate', 'setblock', 'summon', 'tag']
# As persist: not summonable alone, gone at once, or a fight that breaks the area.
SKIP_ENTITIES = {'player', 'fishing_bobber', 'lightning_bolt', 'ender_dragon', 'wither'}
# The scenario's places, as `{N}` in a command: the barrel, the chest, the stone block, air above the floor.
PLACES = [[7, 0, 7], [9, 0, 7], [7, 0, 9], [9, 1, 9]]
BLOCKS = ['air', 'stone', 'chest', 'barrel', 'oak_sign', 'water', 'furnace[lit=true]', 'hopper', 'white_banner',
          'oak_slab[type=top]', 'sand', 'decorated_pot', 'beehive', 'jukebox', 'lectern']
PATHS = ['Items', 'Items[0]', 'Items[0].count', 'Items[{Slot:1b}].components', 'CustomName', 'Health', 'Pos[1]', 'Tags',
         'Tags[0]', 'fz', 'fz.a[0]', 'Item.components', 'Item.count', 'Motion', '{}', 'attributes[0].base']
TAGS = ['1', '1b', '"x"', '{a:1}', '[1,2]', '5.5f', '[I;1,2]', '2s', '[]']
COMPOUNDS = ['{}', '{fz:1}', '{CustomName:"n"}', '{Tags:["a"]}', '{Items:[]}', '{Health:5f}', '{NoGravity:1b}',
             '{Item:{id:"minecraft:dirt",count:2}}', '{fz:{a:[1,2]}}']
SLOTS = ['container.0', 'container.1', 'container.26', 'container.*', 'weapon.mainhand', 'weapon.offhand', 'armor.head',
         'armor.*', 'contents', 'hotbar.0', 'inventory.0', 'container.99']
TABLES = ['minecraft:blocks/stone', 'minecraft:chests/simple_dungeon', 'minecraft:entities/pig', 'minecraft:fz_missing',
          '{pools:[{rolls:1,entries:[{type:"minecraft:item",name:"minecraft:apple"}]}]}',
          '{pools:[{rolls:1,entries:[{type:"minecraft:item",name:"minecraft:apple",functions:[{type:'
          '"minecraft:set_count",count:4}]}]}]}']
MODIFIERS = ['{type:"minecraft:set_count",count:2}', '{type:"minecraft:set_damage",damage:0.5}',
             '{type:"minecraft:set_components",components:{"minecraft:max_stack_size":16}}',
             '{type:"minecraft:set_name",name:"Rock"}', '{type:"minecraft:set_enchantments",enchantments:'
             '{"minecraft:sharpness":3}}', '[{type:"minecraft:set_count",count:2},{type:"minecraft:set_custom_data",'
             'tag:{a:1b}}]', '{type:"minecraft:toggle_tooltips",toggles:{"minecraft:enchantments":false}}',
             '{type:"minecraft:set_lore",lore:["one","two"],mode:"replace_all"}', 'minecraft:fz_missing']
TEXTS = ['"fz"', '{text:"a",color:"red"}', '["a",{text:"b",bold:true}]']
PROVIDERS = {'context_int_provider': ['1', '5', '{type:"minecraft:uniform",min:1,max:3}',
                                      '{type:"minecraft:constant",value:2}'],
             'context_float_provider': ['1.0', '0.5', '{type:"minecraft:uniform",min:0.0,max:1.0}']}
FIXED = {'bool': ['true', 'false'], 'nbt_path': PATHS, 'nbt_tag': TAGS, 'nbt_compound_tag': COMPOUNDS,
         'slot_source': SLOTS, 'item_slot': [s for s in SLOTS if '*' not in s], 'loot_table': TABLES,
         'loot_modifier': MODIFIERS, 'component': TEXTS, 'int_range': ['1', '0..5', '..3', '2..', '-1..1'],
         'float_range': ['0.5', '0..1.5', '..3.0'], 'time': ['0', '1', '5t', '1s', '2d'],
         'entity_anchor': ['eyes', 'feet'], 'swizzle': ['x', 'xz', 'xyz'], 'vec2': ['0 0', '90 -45'],
         'rotation': ['0 0', '90 -45', '-180 90'],
         'heightmap': ['world_surface', 'motion_blocking', 'ocean_floor', 'motion_blocking_no_leaves'],
         'loot_predicate': ['{type:"minecraft:random_chance",chance:1.0}',
                            '{condition:"minecraft:random_chance",chance:1.0}'], **PROVIDERS}
# A `resource` argument's registry -> its list in command-registries.json.
REGISTRIES = {'minecraft:attribute': 'attributes', 'minecraft:mob_effect': 'effects',
              'minecraft:enchantment': 'enchantments', 'minecraft:damage_type': 'damage_types',
              'minecraft:entity_type': 'summonable', 'minecraft:worldgen/biome': 'biomes'}


def load_tree():
    root = Path(os.environ.get('HORIZON_TEST_RESOURCES', lab.RESOURCES))
    data.load_prototypes()
    TREE.update(json.loads((root / 'commands.json').read_text(encoding='utf-8')))
    lists = json.loads((root / 'command-registries.json').read_text(encoding='utf-8'))
    NAMES.update({key: sorted(value) for key, value in lists.items()
                  if isinstance(value, list) and all(isinstance(v, str) for v in value)})
    registries = json.loads((root / 'registries.json').read_text(encoding='utf-8'))
    for key, name in (('minecraft:particle_type', 'particles'), ('minecraft:block', 'all_blocks')):
        NAMES[name] = sorted(registries.get(key, {}).get('entries', {}))


def number(rng, properties, values):
    low, high = properties.get('min'), properties.get('max')
    inside = [v for v in values if (low is None or v >= low) and (high is None or v <= high)]
    edges = [v for v in (low, high) if v is not None]
    if low is not None and rng.random() < 0.08:
        return low - 1  # below the limit: both refuse, with the same words
    return rng.choice(inside + edges)


def argument(rng, node, seed):
    """Text for one argument node, or None when its parser has no generator (or names a player)."""
    parser = node['parser'].split(':', 1)[1]
    properties = node.get('properties') or {}
    if parser in FIXED:
        return rng.choice(FIXED[parser])
    if parser in ('integer', 'long'):
        return str(number(rng, properties, [0, 1, 2, 3, 7, 64, 255]))
    if parser in ('float', 'double'):
        return str(number(rng, properties, [0.0, 0.5, 1.0, 2.5, -1.0]))
    if parser == 'string':
        return {'word': 'fz', 'phrase': '"f z"'}.get(properties.get('type'), 'fz text')
    if parser in ('block_pos', 'vec3'):
        return '{%d}' % rng.randrange(len(PLACES))
    if parser == 'entity':
        if properties.get('type') == 'players':
            return None
        own = ['{u0}', '{u1}', '{u2}']
        if properties.get('amount') == 'single':
            return rng.choice(own + [f'@e[tag=fz{seed},limit=1,sort=nearest]'])
        return rng.choice(own + [f'@e[tag=fz{seed}]', f'@e[tag=fz{seed},type=minecraft:pig]',
                                 f'@e[tag=fz{seed},type=!minecraft:item]'])
    if parser == 'resource_location':
        return f'fz:s{seed}'
    if parser in ('item_stack', 'item_predicate'):
        name = rng.choice(sorted(data.PROTOTYPES) or ['minecraft:stone'])
        patched = data.patch(rng, data.PROTOTYPES.get(name, {})) if rng.random() < 0.6 else {}
        if parser == 'item_stack':
            return stack_text({'item': name, 'nbt': patched}, False)
        return data.predicate(rng, name, 1, patched, data.PROTOTYPES.get(name, {}))[1]
    if parser == 'block_state':
        return rng.choice(NAMES['all_blocks']) if NAMES.get('all_blocks') and rng.random() < 0.5 else rng.choice(BLOCKS)
    if parser == 'block_predicate':
        return rng.choice(BLOCKS + ['#minecraft:logs', '#minecraft:fz_missing', 'barrel{Items:[]}'])
    if parser == 'particle':
        return rng.choice(NAMES.get('particles') or ['minecraft:flame'])
    if parser in ('resource', 'resource_or_tag'):
        names = NAMES.get(REGISTRIES.get(properties.get('registry')))
        if not names:
            return None
        if properties.get('registry') == 'minecraft:entity_type':
            names = [n for n in names if n.split(':')[-1] not in SKIP_ENTITIES]
        if parser == 'resource_or_tag' and rng.random() < 0.2:
            return '#minecraft:is_forest'
        return rng.choice(names)
    return None


def command(rng, seed):
    """(command text, its literals) for one walk of the tree to a node that runs, or None after 30 dead ends."""
    for _ in range(30):
        root = rng.choice([r for r in ROOTS if r in TREE.get('children', {})] or [None])
        if root is None:
            return None
        node, words, literals, own = TREE['children'][root], [root], [root], False
        while True:
            # Command storage outlives a scenario and is one for the whole batch: a write under a name the seed gives
            # would reach the same seed's other shrink candidates.
            children = {k: v for k, v in (node.get('children') or {}).items() if k != 'storage'}
            if root == 'execute' and len(words) == 1:  # the conditions run by themselves; the rest only redirects
                children = {k: v for k, v in children.items() if k in ('if', 'unless')}
            # `own`: the command names one of the scenario's blocks or entities, so it cannot fall back on its
            # sender (`/kill`, `/effect clear`), the player every scenario of the batch shares.
            if node.get('executable') and own and (not children or rng.random() < 0.35):
                return ' '.join(words), ' '.join(literals[:4])
            if not children:
                break
            name = rng.choice(sorted(children))
            node = children[name]
            if node['type'] == 'literal':
                words.append(name)
                literals.append(name)
                continue
            text = argument(rng, node, seed)
            if text is None:
                break
            own = own or node['parser'] in ('minecraft:entity', 'minecraft:block_pos', 'minecraft:vec3')
            words.append(text)
    return None


def commands(rng, seed):
    """A fixture of blocks and entities, two to five commands from the tree, then every fixture read."""
    tags = f'["fz{seed}"]'
    steps = [
        {'k': 'block', 'block': 'barrel', 'at': PLACES[0],
         'nbt': {'Items': '[{Slot:0b,id:"minecraft:stone",count:3},{Slot:1b,id:"minecraft:iron_pickaxe",count:1,'
                          'components:{"minecraft:damage":5}}]'}},
        {'k': 'block', 'block': 'chest', 'at': PLACES[1]}, {'k': 'block', 'block': 'stone', 'at': PLACES[2]},
        {'k': 'summon', 'type': 'armor_stand', 'at': [5.5, 0, 5.5], 'n': 0, 'nbt': {'Tags': tags, 'NoGravity': '1b'}},
        {'k': 'summon', 'type': 'pig', 'at': [5.5, 0, 9.5], 'n': 1,
         'nbt': {'Tags': tags, 'NoAI': '1b', 'PersistenceRequired': '1b'}},
        {'k': 'summon', 'type': 'item', 'at': [9.5, 0, 5.5], 'n': 2,
         'nbt': {'Tags': tags, 'Item': '{id:"minecraft:stone",count:5}', 'PickupDelay': '32767s', 'NoGravity': '1b'}}]
    for _ in range(rng.randint(2, 5)):
        made = command(rng, seed)
        if made:
            steps.append({'k': 'cmd', 'cmd': '/' + made[0], 'at': PLACES, 'uuids': True, 'subject': f'cmd/{made[1]}'})
    reads = [{'k': 'read_block', 'at': PLACES[0], 'subject': 'state/barrel'},
             {'k': 'read_block', 'at': PLACES[1], 'subject': 'state/chest'},
             {'k': 'read_entity', 'n': 0, 'subject': 'state/armor_stand'},
             {'k': 'read_entity', 'n': 1, 'subject': 'state/pig'}, {'k': 'read_entity', 'n': 2, 'subject': 'state/item'}]
    steps += reads
    if rng.random() < 0.3:
        steps += [{'k': 'reload'}, *reads]
    return {'family': 'commands', 'seed': seed, 'steps': steps}
