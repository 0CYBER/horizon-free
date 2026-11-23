"""Item-stack and block-entity-data families for tools/parity_fuzz.py (#20446).

`items`: one stack with a component patch reaches a container slot or an item entity by one route, is read, tested
with item predicates, and then copied, moved by a hopper, dropped, edited or saved and loaded. A patch entry is equal
to the item's prototype, different from it, a removal of a prototype component, or a removal of one the prototype
lacks. Items, prototypes and half of the differing values are vanilla's own (the prepared `items.json`: every item,
and for each component every value some item's prototype gives it), not a list here.

`blockdata`: one block entity with optional and nested fields `persist` never sets (it changes only the scalars a
bare block saves), some with another NBT type than vanilla saves, set by `/setblock` or by `/data merge`, read before
and after a reload.
"""
import json
import os
import re
from pathlib import Path

try:
    from . import lab
    from .parity_fuzz import patch_nbt
except ImportError:
    import lab
    from parity_fuzz import patch_nbt

PROTOTYPES = {}  # 'minecraft:stone' -> {component: JSON value}, filled by load_prototypes
GAME = {}  # component -> every value some item's prototype gives it, as SNBT: values to set on any other item


def load_prototypes():
    root = Path(os.environ.get('HORIZON_TEST_RESOURCES', lab.RESOURCES))
    PROTOTYPES.update(json.loads((root / 'items.json').read_text(encoding='utf-8')))
    for prototype in PROTOTYPES.values():
        for key, value in prototype.items():
            GAME.setdefault(key, set()).add(to_snbt(value))
    GAME.update({key: sorted(values) for key, values in GAME.items()})


def to_snbt(value):
    """A prototype's JSON value as SNBT text."""
    if isinstance(value, bool):
        return 'true' if value else 'false'
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return f'{value!r}d'
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return '[' + ','.join(to_snbt(v) for v in value) + ']'
    return '{' + ','.join(f'{json.dumps(k)}:{to_snbt(v)}' for k, v in value.items()) + '}'


# Items worth a stack of their own: damageable, stackable to 1, 16 and 64, containers, books, and items whose
# prototype carries a component of their own. Every other scenario takes any item of the prototypes instead.
ITEMS = ['apple', 'armor_stand', 'axolotl_bucket', 'bee_nest', 'bow', 'bundle', 'carved_pumpkin', 'chest', 'compass',
         'crossbow', 'debug_stick', 'decorated_pot', 'diamond_sword', 'egg', 'elytra', 'enchanted_book', 'ender_pearl',
         'filled_map', 'firework_rocket', 'firework_star', 'goat_horn', 'golden_apple', 'iron_pickaxe',
         'knowledge_book', 'leather_chestplate', 'lingering_potion', 'mace', 'music_disc_cat', 'mushroom_stew',
         'oak_sign', 'ominous_bottle', 'pig_spawn_egg', 'player_head', 'potion', 'shield', 'shulker_box', 'stone',
         'suspicious_stew', 'tipped_arrow', 'totem_of_undying', 'trident', 'white_banner', 'wolf_armor',
         'writable_book', 'written_book']

STONE = '{id:"minecraft:stone",count:3}'
PICKAXE = '{id:"minecraft:iron_pickaxe",count:1,components:{"minecraft:damage":0}}'
# Component values to set beside the game's own (GAME), as SNBT: the empty and the smallest forms, fields equal to
# their codec's default, a nested stack with a patch of its own. A player's name is never set in a profile: vanilla
# would look it up online.
VALUES = {
    'minecraft:attribute_modifiers': [
        '[]', '[{type:"minecraft:attack_damage",amount:3.0d,id:"minecraft:base_attack_damage",operation:"add_value",'
        'slot:"mainhand"}]', '[{type:"minecraft:armor",amount:1.0d,id:"minecraft:fz",operation:"add_value"}]',
        '[{type:"minecraft:armor",amount:1,id:"minecraft:fz",operation:"add_value",slot:"any"}]'],
    'minecraft:banner_patterns': ['[]', '[{pattern:"minecraft:creeper",color:"red"}]'],
    'minecraft:base_color': ['"red"', '"white"'],
    'minecraft:bees': ['[]', '[{entity_data:{id:"minecraft:bee"},ticks_in_hive:5,min_ticks_in_hive:600}]'],
    'minecraft:block_entity_data': ['{id:"minecraft:chest"}', '{id:"minecraft:chest",Items:[]}'],
    'minecraft:block_state': ['{}', '{lit:"true"}'],
    'minecraft:break_sound': ['"minecraft:entity.item.break"', '"minecraft:block.glass.break"'],
    'minecraft:bucket_entity_data': ['{}', '{NoAI:1b}'],
    'minecraft:bundle_contents': ['[]', '[{id:"minecraft:stone",count:32}]', f'[{PICKAXE}]'],
    'minecraft:charged_projectiles': ['[]', '[{id:"minecraft:arrow",count:1}]',
                                      '[{id:"minecraft:arrow",count:1,components:{"minecraft:rarity":"common"}}]'],
    'minecraft:consumable': ['{}', '{consume_seconds:1.6f,animation:"eat",has_consume_particles:true}',
                             '{consume_seconds:0.5f,on_consume_effects:[]}'],
    'minecraft:container': ['[]', f'[{{slot:0,item:{STONE}}}]', f'[{{slot:0,item:{PICKAXE}}}]',
                            '[{slot:1,item:{id:"minecraft:stone",count:1,components:{"minecraft:max_stack_size":64,'
                            '"!minecraft:lore":{}}}}]'],
    'minecraft:container_loot': ['{loot_table:"minecraft:chests/simple_dungeon"}',
                                 '{loot_table:"minecraft:chests/simple_dungeon",seed:0L}',
                                 '{loot_table:"minecraft:chests/simple_dungeon",seed:42L}'],
    'minecraft:custom_data': ['{}', '{a:1b}', '{a:1}', '{a:{b:[1,2]},c:"x"}'],
    'minecraft:custom_model_data': ['{}', '{floats:[1.0f]}', '{strings:["a"],flags:[true]}'],
    'minecraft:custom_name': ['"fz"', '{text:"fz"}', '{text:"fz",color:"red",italic:false}', '""'],
    'minecraft:damage': ['0', '1', '7', '250', '3.7f'],
    'minecraft:damage_resistant': ['{types:"#minecraft:is_fire"}'],
    'minecraft:death_protection': ['{}', '{death_effects:[]}'],
    'minecraft:debug_stick_state': ['{}'],
    'minecraft:dyed_color': ['255', '16711680', '[1.0f,0.0f,0.0f]'],
    'minecraft:enchantable': ['{value:1}', '{value:14}'],
    'minecraft:enchantment_glint_override': ['true', 'false', '1b'],
    'minecraft:enchantments': ['{}', '{"minecraft:sharpness":2}', '{"minecraft:unbreaking":1,"minecraft:mending":1}'],
    'minecraft:entity_data': ['{id:"minecraft:pig"}', '{id:"minecraft:pig",NoAI:1b}'],
    'minecraft:equippable': ['{slot:"head"}', '{slot:"chest",dispensable:false}',
                             '{slot:"head",dispensable:true,swappable:true,damage_on_hurt:true}'],
    'minecraft:firework_explosion': ['{shape:"small_ball"}', '{shape:"small_ball",colors:[I;],fade_colors:[I;],'
                                     'has_trail:false,has_twinkle:false}'],
    'minecraft:fireworks': ['{}', '{flight_duration:1}', '{flight_duration:1,explosions:[]}',
                            '{explosions:[{shape:"star",colors:[I;255]}]}'],
    'minecraft:food': ['{nutrition:2,saturation:0.5f}', '{nutrition:2,saturation:0.5f,can_always_eat:false}',
                       '{nutrition:0,saturation:0.0f,can_always_eat:true}'],
    'minecraft:glider': ['{}'],
    'minecraft:instrument': ['"minecraft:ponder_goat_horn"', '"minecraft:sing_goat_horn"'],
    'minecraft:intangible_projectile': ['{}'],
    'minecraft:item_model': ['"minecraft:stone"', '"minecraft:fz"'],
    'minecraft:item_name': ['"fz"', '{translate:"block.minecraft.stone"}'],
    'minecraft:jukebox_playable': ['"minecraft:cat"'],
    'minecraft:lock': ['{}', '{items:"minecraft:stick"}'],
    'minecraft:lodestone_tracker': ['{}', '{tracked:true}', '{tracked:false}',
                                    '{target:{pos:[I;1,2,3],dimension:"minecraft:overworld"}}'],
    'minecraft:lore': ['[]', '["a"]', '[{text:"a",color:"gold"},""]'],
    'minecraft:map_color': ['4603950', '255'],
    'minecraft:map_id': ['0', '3'],
    'minecraft:max_damage': ['1', '250', '300'],
    'minecraft:max_stack_size': ['1', '16', '64', '99'],
    'minecraft:minimum_attack_charge': ['0.0f', '1.0f', '0.5f'],
    'minecraft:note_block_sound': ['"minecraft:block.note_block.bell"'],
    'minecraft:ominous_bottle_amplifier': ['0', '2', '4'],
    'minecraft:pot_decorations': ['[]', '["minecraft:angler_pottery_sherd"]',
                                  '["minecraft:brick","minecraft:brick","minecraft:brick","minecraft:brick"]'],
    'minecraft:potion_contents': ['{}', '{potion:"minecraft:swiftness"}', '"minecraft:swiftness"',
                                  '{custom_color:255,custom_effects:[{id:"minecraft:speed",duration:200}]}',
                                  '{custom_effects:[{id:"minecraft:speed",amplifier:0b,duration:200,ambient:false,'
                                  'show_particles:true,show_icon:true}]}'],
    'minecraft:potion_duration_scale': ['1.0f', '0.25f'],
    'minecraft:profile': ['{}'],
    'minecraft:rarity': ['"common"', '"uncommon"', '"epic"'],
    'minecraft:recipes': ['[]', '["minecraft:stick"]'],
    'minecraft:repair_cost': ['0', '1', '39'],
    'minecraft:repairable': ['{items:"minecraft:stick"}', '{items:["minecraft:stick"]}', '{items:"#minecraft:planks"}'],
    'minecraft:stored_enchantments': ['{}', '{"minecraft:sharpness":2}'],
    'minecraft:suspicious_stew_effects': ['[]', '[{id:"minecraft:speed"}]', '[{id:"minecraft:speed",duration:160}]',
                                          '[{id:"minecraft:speed",duration:100}]'],
    'minecraft:tool': ['{rules:[]}', '{rules:[{blocks:"minecraft:stone",speed:2.0f}],damage_per_block:2}',
                       '{rules:[],default_mining_speed:1.0f,damage_per_block:1,can_destroy_blocks_in_creative:true}'],
    'minecraft:tooltip_display': ['{}', '{hide_tooltip:true}', '{hidden_components:["minecraft:enchantments"]}',
                                  '{hide_tooltip:false,hidden_components:[]}'],
    'minecraft:trim': ['{material:"minecraft:iron",pattern:"minecraft:coast"}'],
    'minecraft:unbreakable': ['{}'],
    'minecraft:use_cooldown': ['{seconds:1.0f}', '{seconds:1.0f,cooldown_group:"minecraft:fz"}'],
    'minecraft:use_remainder': ['{id:"minecraft:bowl"}', '{id:"minecraft:bowl",count:1}',
                                '{id:"minecraft:iron_pickaxe",components:{"minecraft:damage":0}}'],
    'minecraft:weapon': ['{}', '{item_damage_per_attack:1,disable_blocking_for_seconds:0.0f}',
                         '{item_damage_per_attack:2}'],
    'minecraft:writable_book_content': ['{}', '{pages:[]}', '{pages:["a"]}', '{pages:[{raw:"a"}]}',
                                        '{pages:[{raw:"a",filtered:"b"}]}'],
    'minecraft:written_book_content': ['{title:"t",author:"a"}',
                                       '{title:"t",author:"a",generation:0,pages:[],resolved:false}',
                                       '{title:"t",author:"a",generation:2,pages:["p"],resolved:true}'],
}
# Item sub-predicates (`component~value`), by the predicate's name.
PARTIAL = {
    'minecraft:attribute_modifiers': ['{modifiers:{size:{min:1}}}',
                                      '{modifiers:{contains:[{attribute:"minecraft:attack_damage"}]}}'],
    'minecraft:bundle_contents': ['{items:{size:{min:1}}}', '{items:{contains:[{items:"minecraft:stone"}]}}'],
    'minecraft:container': ['{items:{size:{min:1}}}', '{items:{contains:[{items:"minecraft:stone"}]}}'],
    'minecraft:custom_data': ['{}', '{a:1b}', '{a:{b:[1]}}'],
    'minecraft:damage': ['{damage:0}', '{damage:{min:1}}', '{durability:{min:1}}', '{durability:{max:249}}'],
    'minecraft:enchantments': ['[]', '[{enchantments:"minecraft:sharpness"}]', '[{levels:{min:2}}]'],
    'minecraft:fireworks': ['{flight_duration:1}', '{explosions:{size:0}}'],
    'minecraft:potion_contents': ['"minecraft:swiftness"', '["minecraft:swiftness","minecraft:water"]'],
    'minecraft:stored_enchantments': ['[]', '[{enchantments:"minecraft:sharpness"}]'],
    'minecraft:writable_book_content': ['{pages:{size:0}}', '{pages:{contains:["a"]}}'],
    'minecraft:written_book_content': ['{author:"a"}', '{generation:{min:1}}', '{resolved:false}'],
}
CONTAINERS = ['barrel', 'barrel', 'chest', 'dispenser', 'shulker_box']
# False (`--ordinary`): only what a command or a played world gives. No wrong tag types, unknown ids, counts over the
# limit or hand-written NBT of a saved stack, whose differences otherwise lead every round.
EDGES = True


def patch(rng, prototype):
    """One to three patch entries: {component: SNBT value, or None for a removal}."""
    out = {}
    for _ in range(rng.randint(1, 3)):
        kind = rng.choice(['equal', 'equal', 'changed', 'changed', 'removed', 'absent'] if EDGES
                          else ['equal', 'changed', 'changed', 'changed', 'removed'])
        if kind == 'equal' and prototype:
            key = rng.choice(sorted(prototype))
            out[key] = to_snbt(prototype[key])
        elif kind == 'removed' and prototype:
            out[rng.choice(sorted(prototype))] = None
        elif kind == 'absent':
            out[rng.choice(sorted(set(VALUES) - set(prototype)))] = None
        else:
            # Half from the game's data: another item's value for a component, whatever the component is.
            pool = GAME if GAME and (not EDGES or rng.random() < 0.5) else VALUES
            key = rng.choice(sorted(pool))
            out[key] = rng.choice(pool[key])
    return out


def stack_limit(prototype, patched):
    """(how many of the prototype under this patch make one stack, whether vanilla takes such a stack at all): a patch
    can lower the limit or remove it (then 1), and an item with a `max_damage` may not stack."""
    def effective(key):
        return patched[key] if key in patched else prototype.get(key)
    size = effective('minecraft:max_stack_size')
    number = re.match(r'-?\d+', str(size)) if size is not None else None
    size = int(number[0]) if number else 1
    return size, not (effective('minecraft:max_damage') is not None and size > 1)


def predicate(rng, item, count, patched, prototype):
    """(kind, an item predicate for the stack): exact value, presence, absence, a sub-predicate, or the count."""
    kind = rng.choice(['exact', 'exact', 'exists', 'not', 'partial', 'count'])
    keys = sorted(patched) if patched and rng.random() < 0.6 else sorted(set(patched) | set(prototype)) or sorted(VALUES)
    key = rng.choice(keys)
    if kind == 'exact':
        values = ([patched[key]] if patched.get(key) is not None else []) \
            + ([to_snbt(prototype[key])] if key in prototype else []) + (VALUES.get(key, []) if EDGES else [])
        test = f'{key}={rng.choice(values)}' if values else key
    elif kind == 'exists':
        test = key
    elif kind == 'not':
        test = f'!{key}'
    elif kind == 'partial':
        key = rng.choice(sorted(set(PARTIAL) & set(keys)) or sorted(PARTIAL))
        test = f'{key}~{rng.choice(PARTIAL[key])}'
    else:
        test = rng.choice(['count=1', f'count={count}', 'count~{min:2}', 'count~{max:1}'])
    return kind, f'{rng.choice(["*", item])}[{test}]'


def items(rng, seed):
    """One stack with a component patch, by one route, read and tested, then moved, changed or saved and loaded."""
    name = 'minecraft:' + (rng.choice(sorted(PROTOTYPES)).split(':', 1)[1] if PROTOTYPES and rng.random() < 0.5
                           else rng.choice(ITEMS))
    prototype = PROTOTYPES.get(name, {})
    patched = patch(rng, prototype)
    # Command-built stacks first: decoding a saved stack (`nbt`, `merge`, `entity`) is one code path of its own.
    route = rng.choice(['with', 'with', 'with', 'with', 'modify', 'modify', 'loot']
                       + (['nbt', 'merge', 'entity'] if EDGES else ['loot']))
    after = rng.choice([None, 'reload', 'reload', 'copy', 'hopper', 'drop', 'count'] + (['edit'] if EDGES else []))
    if route == 'entity' and after not in (None, 'reload'):
        after = 'reload'
    # Within the prototype's stack limit, but for one stack in ten: a count vanilla refuses is a comparison too, and
    # a patch can raise the limit.
    limit, valid = stack_limit(prototype, patched)
    if not EDGES and not valid:  # the prototype alone is a stack vanilla takes
        patched.pop('minecraft:max_damage', None)
        patched.pop('minecraft:max_stack_size', None)
        limit = stack_limit(prototype, patched)[0]
    if route == 'modify':  # the stack is placed bare first, and patched where it lies
        limit = min(limit, stack_limit(prototype, {})[0])
    if EDGES and rng.random() < 0.1:
        limit = 64
    count = rng.choice([n for n in ([1, 2] if after == 'hopper' else [1, 1, 2, 16, 64]) if n <= limit] or [1])
    a, hopper, b, c = [7, 2, 7], [7, 1, 7], [7, 0, 7], [9, 0, 7]
    stack = {'k': 'stack', 'how': route, 'at': a, 'item': name, 'count': count, 'nbt': patched, 'setup': True,
             'subject': f'build/{route}'}
    steps = []
    if after == 'hopper':
        # The hopper moves the stack from A above it into B below, which may hold a plain stack of the item to
        # merge with. Placed before the stack exists, so the hopper's own clock starts the same in both engines.
        steps.append({'k': 'block', 'block': 'barrel', 'at': b})
        if rng.random() < 0.6:
            steps.append({'k': 'cmd', 'cmd': f'/item replace block {{0}} container.0 with {name} 1', 'at': [b]})
        steps.append({'k': 'block', 'block': 'hopper[facing=down]', 'at': hopper})
    if route == 'entity':
        steps.append({'k': 'summon', 'type': 'item', 'at': [7.5, 0, 7.5], 'n': 0, 'setup': True,
                      'subject': 'build/entity',
                      'nbt': {'Item': f'{{id:"{name}",count:{count},components:{patch_nbt({"nbt": patched})}}}',
                              'PickupDelay': '32767s', 'NoGravity': '1b'}})
        read = {'k': 'read_entity', 'n': 0, 'path': 'Item', 'subject': route}
        where = '/execute if items entity {u0} contents '
    else:
        container = rng.choice(CONTAINERS)
        if route == 'nbt':
            stack['block'] = container
        else:
            steps.append({'k': 'block', 'block': container, 'at': a})
        if route == 'modify':
            steps.append({'k': 'cmd', 'cmd': f'/item replace block {{0}} container.0 with {name} {count}', 'at': [a],
                          'setup': True, 'subject': 'build/modify'})
        steps.append(stack)
        read = {'k': 'read_block', 'at': a, 'path': 'Items', 'subject': route}
        where = '/execute if items block {0} container.0 '

    def tests(n):
        out = []
        for _ in range(n):
            kind, text = predicate(rng, name, count, patched, prototype)
            out.append({'k': 'cmd', 'cmd': where + text, 'at': [a], 'uuids': route == 'entity',
                        'subject': f'if_items/{kind}'})
        return out

    steps += [read, *tests(rng.randint(1, 2))]
    label = f'{route}>{after}'
    if after == 'reload':
        steps += [{'k': 'reload'}, {**read, 'subject': label}, *tests(1)]
    elif after == 'copy':
        steps += [{'k': 'block', 'block': 'barrel', 'at': c},
                  {'k': 'cmd', 'cmd': '/item replace block {1} container.3 from block {0} container.0', 'at': [a, c],
                   'subject': 'copy'},
                  {'k': 'read_block', 'at': c, 'path': 'Items', 'subject': label}]
    elif after == 'hopper':
        steps += [{'k': 'tick', 'n': 40}, {'k': 'read_block', 'at': b, 'path': 'Items', 'subject': label},
                  {'k': 'read_block', 'at': hopper, 'path': 'Items', 'subject': f'{label}/hopper'},
                  {**read, 'subject': f'{label}/source'}]
    elif after == 'drop':
        steps += [{'k': 'cmd', 'cmd': '/setblock {0} minecraft:air destroy', 'at': [a], 'subject': 'drop'},
                  {'k': 'read_near', 'type': 'item', 'at': a, 'path': 'Item', 'subject': label}]
    elif after == 'count':
        steps += [{'k': 'cmd', 'cmd': '/item modify block {0} container.0 {type:"minecraft:set_count",count:2}',
                   'at': [a], 'subject': 'count'}, {**read, 'subject': label}]
    elif after == 'edit':
        steps += [{'k': 'cmd', 'at': [a], 'subject': 'edit',
                   'cmd': '/data modify block {0} Items[0].components."minecraft:custom_data" set value {e:1}'},
                  {**read, 'subject': label}, *tests(1)]
    return {'family': 'items', 'seed': seed, 'steps': steps}


# ---------------------------------------------------------------------------------------------------------------
# Block-entity data. Values are SNBT: what vanilla saves, the empty form, out-of-range and unknown ones.

SLOTS = ['[{Slot:0b,id:"minecraft:stone",count:3}]',
         '[{Slot:1b,id:"minecraft:iron_pickaxe",count:1,components:{"minecraft:damage":5}}]',
         '[{Slot:0b,id:"minecraft:stone",count:3},{Slot:0b,id:"minecraft:dirt",count:1}]',
         '[{Slot:99b,id:"minecraft:stone",count:1}]', '[{Slot:2b,id:"minecraft:fz_missing",count:1}]',
         '[{Slot:0b,id:"minecraft:stone",count:0}]', '[{Slot:0b,id:"minecraft:stone",count:100}]',
         '[{id:"minecraft:stone",count:1}]', '[{Slot:0,id:"minecraft:stone",count:2b}]']
NAMES = ['"fz"', '{text:"fz",color:"red"}', '{text:"fz",italic:false}', '""']
LOCKS = ['{items:"minecraft:stick"}', '{}', '{components:{"minecraft:custom_name":"k"}}', '{count:{min:2}}']
NAMED = {'CustomName': NAMES, 'lock': LOCKS}
CONTAINER = {**NAMED, 'Items': SLOTS, 'LootTable': ['"minecraft:chests/simple_dungeon"', '"minecraft:fz_missing"'],
             'LootTableSeed': ['42L', '0L']}
FURNACE = {**NAMED, 'Items': SLOTS, 'cooking_time_spent': ['20', '0', '300', '-1'],
           'cooking_total_time': ['200', '0', '100'], 'lit_time_remaining': ['100', '0', '-1'],
           'lit_total_time': ['1600', '0'],
           'RecipesUsed': ['{"minecraft:iron_ingot":2}', '{"minecraft:fz_missing":1}', '{}']}
# A command block never runs here (no `auto`, no signal): its output would reach the player between two reads.
COMMAND = {'CustomName': NAMES, 'Command': ['"time query gametime"', '""'], 'TrackOutput': ['0b', '1b'],
           'SuccessCount': ['3', '0'], 'LastOutput': ['{text:"x"}', '"x"'], 'UpdateLastExecution': ['0b', '1b'],
           'LastExecution': ['20L', '0L'], 'conditionMet': ['1b', '0b']}
TEXT = ['{messages:["hello","world","",""]}', '{messages:["a","","",""],color:"red",has_glowing_text:1b}',
        '{messages:[{text:"a",color:"red"},"","",""]}', '{messages:["a","b"]}',
        '{messages:["a","","",""],filtered_messages:["b","","",""]}', '{messages:["a","","",""],color:"fz"}']
SIGN = {'front_text': TEXT, 'back_text': TEXT, 'is_waxed': ['1b', '0b', '2b']}
LISTENER = ['{event_delay:0,selector:{tick:-1L}}', '{event_delay:5,selector:{tick:-1L}}', '{}']
BEES = '{entity_data:{id:"minecraft:bee"},ticks_in_hive:5,min_ticks_in_hive:6000}'
HIVE = {'bees': [f'[{BEES}]', f'[{BEES},{BEES},{BEES},{BEES}]', '[]',
                 '[{entity_data:{},ticks_in_hive:0,min_ticks_in_hive:6000}]',
                 '[{entity_data:{id:"minecraft:bee",CustomName:"b"},ticks_in_hive:0,min_ticks_in_hive:6000}]'],
        'flower_pos': ['[I;1,2,3]', '[I;1,2]', '{X:1,Y:2,Z:3}']}
BRUSHABLE = {'item': ['{id:"minecraft:diamond",count:1}', '{id:"minecraft:stone",count:64}', '{}'],
             'LootTable': ['"minecraft:archaeology/desert_pyramid"'], 'LootTableSeed': ['42L'],
             'hit_direction': ['1', '6', '-1']}
BLOCKS = {
    'barrel': CONTAINER, 'chest': CONTAINER, 'trapped_chest': CONTAINER, 'red_shulker_box': CONTAINER,
    'dispenser': CONTAINER, 'dropper': CONTAINER,
    'hopper': {**CONTAINER, 'TransferCooldown': ['5', '0', '-1', '8']},
    'crafter': {**NAMED, 'Items': SLOTS, 'disabled_slots': ['[I;1,4]', '[I;0]', '[I;9]', '[I;-1,4,4]', '[I;]'],
                'triggered': ['1', '0', '2'], 'crafting_ticks_remaining': ['3', '0', '-1']},
    'furnace': FURNACE, 'smoker': FURNACE, 'blast_furnace': FURNACE,
    'brewing_stand': {**NAMED, 'BrewTime': ['30', '0', '400', '-1'], 'Fuel': ['10', '0', '20', '21'],
                      'Items': ['[{Slot:4b,id:"minecraft:blaze_powder",count:1}]',
                                '[{Slot:0b,id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":'
                                '{potion:"minecraft:water"}}},{Slot:3b,id:"minecraft:nether_wart",count:1}]']},
    'beacon': {**NAMED, 'primary_effect': ['"minecraft:speed"', '"minecraft:regeneration"', '"minecraft:fz_missing"'],
               'secondary_effect': ['"minecraft:regeneration"', '"minecraft:speed"'], 'Levels': ['4']},
    'enchanting_table': {'CustomName': NAMES},
    'command_block': COMMAND, 'repeating_command_block': COMMAND, 'chain_command_block': COMMAND,
    'campfire': {'Items': ['[{Slot:0b,id:"minecraft:beef",count:1},{Slot:3b,id:"minecraft:porkchop",count:1}]',
                           '[{Slot:0b,id:"minecraft:stone",count:1}]', '[{Slot:4b,id:"minecraft:beef",count:1}]',
                           '[{Slot:0b,id:"minecraft:beef",count:2}]'],
                 'CookingTimes': ['[I;5,0,0,9]', '[I;5]', '[I;1,2,3,4,5]', '[I;-1,0,0,0]'],
                 'CookingTotalTimes': ['[I;600,0,0,600]', '[I;10,10,10,10]', '[I;0,0,0,0]']},
    'chiseled_bookshelf': {'Items': ['[{Slot:0b,id:"minecraft:book",count:1},{Slot:5b,id:"minecraft:enchanted_book",'
                                     'count:1}]', '[{Slot:0b,id:"minecraft:stone",count:1}]',
                                     '[{Slot:6b,id:"minecraft:book",count:1}]', '[{Slot:1b,id:"minecraft:book",count:3}]'],
                           'last_interacted_slot': ['5', '-1', '6', '0']},
    'oak_shelf': {'Items': SLOTS[:4], 'align_items_to_bottom': ['1b', '0b', '2b']},
    'beehive': HIVE, 'bee_nest': HIVE,
    'white_banner': {'CustomName': NAMES,
                     'patterns': ['[{pattern:"minecraft:creeper",color:"red"}]', '[]',
                                  '[{pattern:"minecraft:stripe_top",color:"blue"},{pattern:"minecraft:cross",'
                                  'color:"white"}]', '[{pattern:"minecraft:fz_missing",color:"red"}]',
                                  '[{pattern:"minecraft:creeper",color:"fz"}]', '[{pattern:"minecraft:creeper"}]']},
    'decorated_pot': {'sherds': ['{back:{id:"minecraft:angler_pottery_sherd"},front:{id:"minecraft:skull_pottery_sherd"}}',
                                 '{front:{id:"minecraft:brick"}}', '{left:{id:"minecraft:stone"}}', '{}'],
                      'item': ['{id:"minecraft:diamond",count:3}', '{id:"minecraft:stone",count:64}', PICKAXE, '{}'],
                      'LootTable': ['"minecraft:archaeology/desert_pyramid"']},
    'oak_sign': SIGN, 'oak_hanging_sign': SIGN,
    'skeleton_skull': {'custom_name': ['"Bones"', '{text:"b",bold:true}'],
                       'note_block_sound': ['"minecraft:block.note_block.bell"', '"minecraft:fz"', '"fz"']},
    'spawner': {'Delay': ['20', '-1', '0'], 'MinSpawnDelay': ['200', '0', '900'], 'MaxSpawnDelay': ['800', '100'],
                'SpawnCount': ['2', '0'], 'MaxNearbyEntities': ['6', '0'], 'RequiredPlayerRange': ['8', '0', '-1'],
                'SpawnRange': ['4', '0'],
                'SpawnData': ['{entity:{id:"minecraft:pig"}}', '{entity:{}}', '{entity:{id:"minecraft:fz_missing"}}',
                              '{entity:{id:"minecraft:zombie"},custom_spawn_rules:{block_light_limit:[0,15]}}'],
                'SpawnPotentials': ['[{data:{entity:{id:"minecraft:pig"}},weight:1}]', '[]',
                                    '[{data:{entity:{id:"minecraft:pig"}},weight:1},{data:{entity:{id:"minecraft:cow"}},'
                                    'weight:3}]']},
    'jukebox': {'RecordItem': ['{id:"minecraft:music_disc_cat",count:1}', '{id:"minecraft:stone",count:1}',
                               '{id:"minecraft:music_disc_13",count:2}'],
                'ticks_since_song_started': ['5L', '0L', '100000L', '-1L']},
    'lectern': {'Book': ['{id:"minecraft:writable_book",count:1}', '{id:"minecraft:stone",count:1}',
                         '{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":'
                         '{title:"t",author:"a",pages:["p","q"]}}}',
                         '{id:"minecraft:writable_book",count:1,components:{"minecraft:writable_book_content":'
                         '{pages:["a","b","c"]}}}'],
                'Page': ['0', '1', '5', '-1']},
    'sculk_sensor': {'last_vibration_frequency': ['6', '0', '15', '16', '-1'], 'listener': LISTENER},
    'calibrated_sculk_sensor': {'last_vibration_frequency': ['6', '0', '15', '16', '-1'], 'listener': LISTENER},
    'sculk_shrieker': {'warning_level': ['2', '0', '4', '5', '-1'], 'listener': LISTENER},
    'comparator': {'OutputSignal': ['5', '0', '15', '16', '-1']},
    'end_gateway': {'Age': ['5L', '0L', '200L'], 'ExactTeleport': ['1b', '0b'], 'exit_portal': ['[I;4100,300,4100]', '[I;1,2]']},
    'conduit': {'Target': ['[I;1,2,3,4]']},
    'suspicious_sand': BRUSHABLE, 'suspicious_gravel': BRUSHABLE,
    'trial_spawner': {'spawn_data': ['{entity:{id:"minecraft:zombie"}}', '{entity:{}}'],
                      'normal_config': ['"minecraft:trial_chamber/melee/zombie/normal"', '{spawn_range:4,total_mobs:6.0f}'],
                      'required_player_range': ['10', '0'], 'total_mobs_spawned': ['2', '0']},
    'vault': {'config': ['{key_item:{id:"minecraft:stick",count:1},activation_range:4.0d,deactivation_range:4.5d}',
                         '{loot_table:"minecraft:chests/trial_chambers/reward",key_item:{id:"minecraft:trial_key",count:1}}'],
              'server_data': ['{rewarded_players:[[I;1,2,3,4]],items_to_eject:[],total_ejections_needed:0}',
                              '{items_to_eject:[{id:"minecraft:stone",count:1}],total_ejections_needed:1}'],
              'shared_data': ['{display_item:{id:"minecraft:stone",count:1},connected_players:[],'
                              'connected_particles_range:4.5d}']},
    'structure_block': {'name': ['"minecraft:fz"', '""'], 'author': ['"a"'], 'metadata': ['"m"'],
                        'mode': ['"SAVE"', '"LOAD"', '"CORNER"', '"DATA"', '"fz"'], 'posX': ['1', '49', '-49'],
                        'sizeX': ['5', '49', '-1'], 'rotation': ['"CLOCKWISE_90"', '"NONE"'], 'mirror': ['"LEFT_RIGHT"'],
                        'integrity': ['0.5f', '2.0f', '-1.0f'], 'seed': ['5L'], 'ignoreEntities': ['0b'],
                        'showboundingbox': ['0b'], 'showair': ['1b'], 'powered': ['1b']},
    'jigsaw': {'name': ['"minecraft:fz"'], 'target': ['"minecraft:fz"'], 'pool': ['"minecraft:empty"', '"minecraft:fz"'],
               'final_state': ['"minecraft:stone"', '"minecraft:fz"'], 'joint': ['"rollable"', '"aligned"', '"fz"'],
               'placement_priority': ['3'], 'selection_priority': ['2']},
}
# Fields any block entity loads: its stored components (the ones it takes itself become its own fields) and a key
# no block entity knows.
ANY = {'components': ['{"minecraft:custom_data":{a:1b}}', '{"minecraft:custom_name":"n"}', '{}',
                      f'{{"minecraft:container":[{{slot:0,item:{STONE}}}]}}', '{"minecraft:lock":{items:"minecraft:stick"}}',
                      '{"minecraft:rarity":"epic"}'],
       'fz': ['1', '{a:1b}']}
# Blocks whose item form carries their data: broken by `/setblock ... destroy`, one item to read.
DROPS = {'white_banner', 'decorated_pot', 'red_shulker_box', 'skeleton_skull'}
SUFFIXES = ['', 'b', 's', 'L', 'f', 'd']


def mistype(rng, value):
    """A number with another NBT type or as a string, a string as a number: what a lenient reader must settle."""
    match = re.fullmatch(r'(-?\d+)(?:\.\d+)?([bsLfd]?)', value)
    if match:
        other = rng.choice([s for s in SUFFIXES if s != match.group(2)] + ['"'])
        return f'"{match.group(1)}"' if other == '"' else match.group(1) + other
    return '7' if value.startswith('"') else value


def blockdata(rng, seed):
    """One block entity with one to three optional fields, by /setblock or /data merge; read, reloaded, read."""
    block = rng.choice(sorted(BLOCKS))
    fields = {**BLOCKS[block], **ANY} if EDGES else BLOCKS[block]
    nbt = {}
    for key in sorted(rng.sample(sorted(fields), min(len(fields), rng.randint(1, 3)))):
        # Each pool's first value is the one vanilla saves itself.
        value = rng.choice(fields[key]) if EDGES else fields[key][0]
        nbt[key] = mistype(rng, value) if EDGES and rng.random() < 0.2 else value
    c = [7, 0, 7]
    route = rng.choice(['set', 'set', 'merge'])
    if route == 'set':
        steps = [{'k': 'block', 'block': block, 'at': c, 'nbt': nbt, 'setup': True, 'subject': f'{block}/set'}]
    else:
        steps = [{'k': 'block', 'block': block, 'at': c, 'setup': True},
                 {'k': 'block', 'block': block, 'at': c, 'nbt': nbt, 'merge': True, 'subject': f'{block}/merge'}]
    read = {'k': 'read_block', 'at': c, 'subject': f'{block}/{route}'}
    before, after = rng.choice([0, 0, 1, 3]), rng.choice([1, 2, 5])
    steps += [read] + ([{'k': 'tick', 'n': before}, read] if before else [])
    steps += [{'k': 'reload'}, read, {'k': 'tick', 'n': after}, read]
    if block in DROPS and rng.random() < 0.5:
        steps += [{'k': 'cmd', 'cmd': '/setblock {0} minecraft:air destroy', 'at': [c], 'subject': f'{block}/drop'},
                  {'k': 'read_near', 'type': 'item', 'at': c, 'path': 'Item', 'subject': f'{block}/drop'}]
    return {'family': 'blockdata', 'seed': seed, 'steps': steps}
