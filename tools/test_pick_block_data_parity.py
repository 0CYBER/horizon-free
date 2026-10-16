#!/usr/bin/env python3
"""Compare block entity data on items with vanilla 26.3.

The Java oracle (VanillaPickBlockDataOracle) loads each block entity with
BlockEntity.loadStatic and runs ServerGamePacketListenerImpl.addBlockDataToItem
(Ctrl + Pick Block in creative) on the block's item, then places an item as a
block entity: BlockItem.updateCustomBlockEntityTag (op-only types only for a
game master) and BlockEntity.applyComponentsFromItemStack. The Rust example
(pick_block_data_fixture) runs horizon_client::block_entity_item on the
canonical saved NBT. The picked component patch and the placed block entity's
saveWithFullMetadata are compared for every block entity type that carries data:
containers with items, names, locks and loot tables, signs, banners, command
blocks, spawners, trial spawners, vaults, beehives, pots, skulls, lecterns and
more, picked and placed back, with hand-written patches (data of another type,
op-only data without permission, merges into a filled block entity, leftover
components).
"""
import argparse
import json
try:
    from . import interaction_lab as lib
except ImportError:
    import interaction_lab as lib

POS = 'x:0,y:64,z:0'
STONE = '{id:"minecraft:stone",count:5}'
NAME = 'CustomName:"Box"'
PIG = 'SpawnData:{entity:{id:"minecraft:pig"}}'
# A block entity that spawns from nothing: what loading it saves (a spawner's
# potentials and a trial spawner's next spawn come from its first spawn data).
FRESH = {'mob_spawner': PIG, 'trial_spawner': 'spawn_data:{entity:{}}'}


def be(type_id, body=''):
    return '{id:"minecraft:%s",%s%s}' % (type_id, POS, ',' + body if body else '')


def items(*slots):
    return 'Items:[%s]' % ','.join('{Slot:%db,id:"minecraft:%s",count:%d}' % s for s in slots)


# (block, block entity type, saved NBT bodies to pick and place back).
SAMPLES = [
    ('chest', 'chest', ['', items((0, 'stone', 5), (26, 'diamond', 1)), NAME, 'lock:{items:"minecraft:diamond"}',
                        'LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L',
                        'LootTable:"minecraft:chests/igloo_chest"', items((3, 'dirt', 1)) + ',' + NAME]),
    ('trapped_chest', 'trapped_chest', [items((1, 'stone', 2))]),
    ('barrel', 'barrel', [items((5, 'apple', 3)) + ',' + NAME]),
    ('red_shulker_box', 'shulker_box', ['', items((0, 'stone', 64), (13, 'emerald', 2)) + ',' + NAME]),
    ('hopper', 'hopper', ['TransferCooldown:5', items((4, 'stone', 1)) + ',TransferCooldown:2']),
    ('dispenser', 'dispenser', [items((8, 'arrow', 16))]),
    ('dropper', 'dropper', [items((0, 'stone', 1)) + ',' + NAME]),
    ('crafter', 'crafter', ['disabled_slots:[I;1,4],triggered:1,crafting_ticks_remaining:3',
                            items((2, 'stick', 1)) + ',disabled_slots:[I;0]']),
    ('furnace', 'furnace', ['cooking_time_spent:20,cooking_total_time:200,lit_time_remaining:100,lit_total_time:1600,'
                            + items((0, 'iron_ore', 3), (1, 'coal', 4)) + ',RecipesUsed:{"minecraft:iron_ingot":2}']),
    ('smoker', 'smoker', [items((2, 'cooked_beef', 1))]),
    ('blast_furnace', 'blast_furnace', [NAME + ',lit_time_remaining:7']),
    ('brewing_stand', 'brewing_stand', ['BrewTime:30,Fuel:10,' + items((4, 'blaze_powder', 1))]),
    # Levels is computed by the beacon's tick, never loaded.
    ('beacon', 'beacon', ['primary_effect:"minecraft:speed",secondary_effect:"minecraft:regeneration"',
                          NAME + ',lock:{items:"minecraft:stick"}']),
    ('enchanting_table', 'enchanting_table', ['', NAME]),
    ('command_block', 'command_block', ['Command:"say hi",auto:1b,powered:1b,conditionMet:1b,TrackOutput:0b,SuccessCount:3',
                                        'Command:"time set day",' + NAME, 'LastOutput:"{\\"text\\":\\"x\\"}"',
                                        'UpdateLastExecution:0b,LastExecution:20L']),
    ('repeating_command_block', 'command_block', ['Command:"say loop",auto:1b']),
    ('campfire', 'campfire', [items((0, 'beef', 1), (3, 'porkchop', 1)) + ',CookingTimes:[I;5,0,0,9],CookingTotalTimes:[I;600,0,0,600]']),
    ('chiseled_bookshelf', 'chiseled_bookshelf', [items((0, 'book', 1), (5, 'enchanted_book', 1)) + ',last_interacted_slot:5']),
    ('oak_shelf', 'shelf', [items((1, 'stone', 1)) + ',align_items_to_bottom:1b']),
    ('beehive', 'beehive', ['', 'bees:[{entity_data:{id:"minecraft:bee"},ticks_in_hive:5,min_ticks_in_hive:600}]',
                            'flower_pos:[I;1,2,3]']),
    ('white_banner', 'banner', ['', 'patterns:[{pattern:"minecraft:creeper",color:"red"}]',
                                'patterns:[{pattern:"minecraft:stripe_top",color:"blue"}],' + NAME]),
    ('decorated_pot', 'decorated_pot', ['', 'sherds:{back:{id:"minecraft:angler_pottery_sherd"},front:{id:"minecraft:skull_pottery_sherd"}}',
                                        'item:{id:"minecraft:diamond",count:3}', 'LootTable:"minecraft:archaeology/desert_pyramid"']),
    ('oak_sign', 'sign', ['', 'front_text:{messages:["hello","world","",""]}',
                          'front_text:{messages:["a","","",""],color:"red",has_glowing_text:1b},back_text:{messages:["","b","",""]},is_waxed:1b']),
    ('oak_hanging_sign', 'hanging_sign', ['back_text:{messages:["x","","",""]}']),
    ('skeleton_skull', 'skull', ['', 'custom_name:"Bones"', 'note_block_sound:"minecraft:block.note_block.bell"']),
    ('spawner', 'mob_spawner', ['Delay:20,SpawnData:{entity:{id:"minecraft:zombie"}},MinSpawnDelay:200,MaxSpawnDelay:800',
                                'SpawnData:{entity:{id:"minecraft:pig"}},SpawnCount:2,RequiredPlayerRange:8']),
    ('trial_spawner', 'trial_spawner', ['spawn_data:{entity:{id:"minecraft:zombie"}}',
                                        'normal_config:"minecraft:trial_chamber/melee/zombie/normal",required_player_range:10,spawn_data:{entity:{}}']),
    ('vault', 'vault', ['', 'config:{key_item:{id:"minecraft:ominous_trial_key",count:1}}']),
    ('jukebox', 'jukebox', ['', 'RecordItem:{id:"minecraft:music_disc_cat",count:1},ticks_since_song_started:40L']),
    ('lectern', 'lectern', ['', 'Book:{id:"minecraft:writable_book",count:1,components:{"minecraft:writable_book_content":{pages:[{raw:"a"},{raw:"b"}]}}},Page:1']),
    ('sculk_sensor', 'sculk_sensor', ['last_vibration_frequency:4']),
    ('creaking_heart', 'creaking_heart', ['']),
    ('bell', 'bell', ['']),
    ('structure_block', 'structure_block', ['name:"minecraft:house",mode:"SAVE",posY:2']),
]

# Patches placed as (block, block entity type, target body, patch SNBT, game master).
DATA = '"minecraft:block_entity_data"'
PATCHES = [
    ('command_block', 'command_block', '', '{%s:{id:"minecraft:command_block",Command:"say op"}}' % DATA, False),
    ('command_block', 'command_block', '', '{%s:{id:"minecraft:chest",Command:"say wrong"}}' % DATA, True),
    ('command_block', 'command_block', 'Command:"old",auto:1b', '{%s:{id:"minecraft:command_block",TrackOutput:0b}}' % DATA, True),
    ('oak_sign', 'sign', '', '{%s:{id:"minecraft:sign",is_waxed:1b}}' % DATA, False),
    ('oak_sign', 'sign', '', '{%s:{id:"minecraft:sign",is_waxed:1b}}' % DATA, True),
    ('chest', 'chest', items((0, 'stone', 1)), '{%s:{id:"minecraft:chest",Items:[{Slot:1b,id:"minecraft:dirt",count:1}]}}' % DATA, True),
    ('chest', 'chest', '', '{"minecraft:container":[{slot:30,item:%s},{slot:2,item:%s}]}' % (STONE, STONE), False),
    ('chest', 'chest', '', '{"minecraft:container_loot":{loot_table:"minecraft:chests/simple_dungeon",seed:7L}}', False),
    ('chest', 'chest', '', '{"minecraft:lore":["a"],"minecraft:custom_name":"Named"}', False),
    ('hopper', 'hopper', 'TransferCooldown:4', '{%s:{id:"minecraft:hopper",TransferCooldown:9}}' % DATA, False),
    ('spawner', 'mob_spawner', PIG, '{%s:{id:"minecraft:mob_spawner",SpawnData:{entity:{id:"minecraft:skeleton"}}}}' % DATA, True),
    ('spawner', 'mob_spawner', PIG, '{%s:{id:"minecraft:mob_spawner",SpawnData:{entity:{id:"minecraft:skeleton"}}}}' % DATA, False),
    ('trial_spawner', 'trial_spawner', 'spawn_data:{entity:{}}', '{%s:{id:"minecraft:trial_spawner",required_player_range:4}}' % DATA, True),
    ('vault', 'vault', '', '{%s:{id:"minecraft:vault",config:{key_item:{id:"minecraft:stick",count:1}}}}' % DATA, False),
    ('decorated_pot', 'decorated_pot', '', '{"minecraft:container":[{slot:1,item:%s}]}' % STONE, False),
    ('white_banner', 'banner', NAME, '{"minecraft:banner_patterns":[{pattern:"minecraft:flower",color:"lime"}]}', False),
    ('beehive', 'beehive', 'bees:[{entity_data:{id:"minecraft:bee"},ticks_in_hive:1,min_ticks_in_hive:2}]', '{}', False),
    ('decorated_pot', 'decorated_pot', '', '{"!minecraft:pot_decorations":{}}', False),
    ('hopper', 'hopper', items((0, 'stone', 1)), '{"minecraft:container":[{slot:4,item:%s},{slot:5,item:%s}]}' % (STONE, STONE), False),
    ('oak_sign', 'sign', '', '{"minecraft:waxed":{},"minecraft:sign_text_back":{messages:["","","","z"]}}', False),
]


def cases():
    out = []
    for block, type_id, bodies in SAMPLES:
        for body in bodies:
            for op in (True, False):
                out.append({'block': 'minecraft:' + block, 'nbt': be(type_id, body),
                            'target': be(type_id, FRESH.get(type_id, '')), 'op': op})
    for block, type_id, target, patch, op in PATCHES:
        out.append({'block': 'minecraft:' + block, 'nbt': be(type_id, FRESH.get(type_id, '')), 'target': be(type_id, target), 'patch': patch, 'op': op})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--limit', type=int, default=4)
    args = parser.parse_args()
    work = lib.work_dir('pick-block-data-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaPickBlockDataOracle')
    all_cases = cases()
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in all_cases))
    vanilla.run('VanillaPickBlockDataOracle', work / 'cases.jsonl', work / 'native.txt')
    lib.cargo_example('horizon-client', 'pick_block_data_fixture', work / 'cases.jsonl', work / 'rust.txt')
    errors = sum(line == 'error' for line in (work / 'native.txt').read_text().split('\n'))
    if errors:
        print(f'{errors} cases fail in vanilla')
    total = lib.compare(all_cases, work / 'native.txt', work / 'rust.txt',
                        lambda case, line: case['block'] + (' patch' if 'patch' in case else ''),
                        lambda case: json.dumps(case), args.limit)
    raise SystemExit(1 if total or errors else 0)


if __name__ == '__main__':
    main()
