#!/usr/bin/env python3
"""A mob's custom name and custom data survive save and load, styled or not, from an egg, a bucket or a summon.

`Entity.saveWithoutId` writes `CustomName` as the whole component (`ComponentSerialization.CODEC`: a plain name
is a string, a styled one a compound) and `Entity.load` reads it back the same way; the stack config of
`SpawnEggItem` and `MobBucketItem.spawn` (`createDefaultStackConfig`) gives the mob the item's `custom_name` and
`custom_data` (saved as `data`). A dispenser spawns a pig from an egg and an axolotl from a bucket, pigs are
summoned with a plain, a styled and a translated name, the world is saved and reloaded
(tools/entity_save_reload.py), and each one's `CustomName` and `data` are read with `/data get` (which saves the
mob again) on vanilla's server and Horizon's.
"""
try:
    from . import entity_save_reload as harness
except ImportError:
    import entity_save_reload as harness

PIG = '@e[type=minecraft:pig,x={x}.5,y=-60,z={z}.5,distance=..1.5,limit=1]'
AXOLOTL = '@e[type=minecraft:axolotl,x=8.5,y=-60,z=5.5,distance=..1.5,limit=1]'

# Dispensers first, so the summons after them give their scheduled ticks time to fire.
SETUP = [
    'setblock 4 -60 4 minecraft:dispenser[facing=south]',
    'item replace block 4 -60 4 container.0 with minecraft:pig_spawn_egg[custom_name={text:"Disp",color:"blue"},'
    'custom_data={d:1b},entity_data={id:"minecraft:pig",NoAI:1b}]',
    'setblock 4 -59 4 minecraft:redstone_block',
    'fill 7 -61 4 9 -59 6 minecraft:glass',
    'setblock 8 -60 5 minecraft:water',
    'setblock 8 -59 5 minecraft:air',
    'setblock 8 -60 4 minecraft:dispenser[facing=south]',
    'item replace block 8 -60 4 container.0 with minecraft:axolotl_bucket[custom_name={text:"Axo",color:"gold"},'
    'custom_data={b:1b},bucket_entity_data={NoAI:1b}]',
    'setblock 8 -59 4 minecraft:redstone_block',
    'summon minecraft:pig 12.5 -60 4.5 {NoAI:1b,CustomName:"Plain"}',
    'summon minecraft:pig 15.5 -60 4.5 {NoAI:1b,CustomName:{text:"Sum",color:"green"}}',
    'summon minecraft:pig 18.5 -60 4.5 {NoAI:1b,CustomName:{translate:"entity.minecraft.pig",italic:true}}',
]
PROBES = [
    ('egg pig CustomName', f'data get entity {PIG.format(x=4, z=5)} CustomName'),
    ('egg pig data', f'data get entity {PIG.format(x=4, z=5)} data'),
    ('bucket axolotl CustomName', f'data get entity {AXOLOTL} CustomName'),
    ('bucket axolotl data', f'data get entity {AXOLOTL} data'),
    ('plain CustomName', f'data get entity {PIG.format(x=12, z=4)} CustomName'),
    ('styled CustomName', f'data get entity {PIG.format(x=15, z=4)} CustomName'),
    ('translated CustomName', f'data get entity {PIG.format(x=18, z=4)} CustomName'),
]


def main():
    return harness.main('mob-name', SETUP, PROBES, __doc__)


if __name__ == '__main__':
    raise SystemExit(main())
