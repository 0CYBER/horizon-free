"""`armor stand thaw`: an armor stand's `TicksFrozen` thaws and builds as `LivingEntity.aiStep` runs it."""

GROUP = 'armor stand thaw'

# (summon NBT, block under the stand's feet and body)
STANDS = [
    ('TicksFrozen:60', None),  # in the open: two a tick
    ('TicksFrozen:60,NoGravity:1b', None),  # no physics, still thaws
    ('TicksFrozen:60,Marker:1b', None),
    ('TicksFrozen:1', None),  # thaws to 0, and the key goes
    ('', 'powder_snow'),  # builds one a tick
    ('TicksFrozen:139', 'powder_snow'),  # up to getTicksRequiredToFreeze (140)
    ('TicksFrozen:60,equipment:{feet:{id:"leather_boots",count:1}}', 'powder_snow'),  # freeze_immune_wearables
    ('TicksFrozen:60,NoGravity:1b', 'powder_snow'),  # no block effects reach it: thaws
    ('TicksFrozen:60,equipment:{head:{id:"leather_helmet",count:1}}', 'powder_snow'),
    ('TicksFrozen:60,equipment:{chest:{id:"iron_chestplate",count:1}}', 'powder_snow'),  # iron: freezes
    ('TicksFrozen:60', 'head_snow'),  # snow at its head only: entityInside wants the feet in it
    ('TicksFrozen:100,Invulnerable:1b', 'fire'),  # FIRE_IGNITE -> igniteForTicks -> clearFreeze
    # fireIgnite only counts -5 up, but a fire block applies CLEAR_FREEZE itself.
    ('TicksFrozen:100,Invulnerable:1b,Fire:-5s', 'fire'),
]


# Burning stands meeting powder snow: (summon NBT, snow cells as y offsets).
BURNING = [
    ('Fire:100s,Invulnerable:1b', (0, 1)),  # in it: put out, and both blocks melt (mobGriefing)
    ('Fire:100s,Invulnerable:1b', (1,)),  # head only: still put out, the block melts
    ('Fire:100s,Invulnerable:1b,NoGravity:1b', (0, 1)),  # no physics: keeps burning, the snow stays
]


def cases(cmd, setup):
    """`LivingEntity.aiStep`'s freezing step after `applyEffectsFromBlocks`: powder snow's
    `InsideBlockEffectType.FREEZE` adds one while `canFreeze`, otherwise `TicksFrozen` drops by two
    (never below 0, and `saveWithoutId` writes it only while positive). Powder snow freezes a living
    entity only with its feet in it, and a fire block igniting the stand clears the freeze
    (`igniteForTicks`). A burning stand the snow meets is put out, and melts that snow
    (`runBefore(EXTINGUISH)`). Each stand is read after 1,
    2 and 20 ticks."""
    x0, y0, z0 = 7520, 250, 7520
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': [[x0 >> 4, z0 >> 4]]},
           cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} stone', False)]
    for n, (nbt, block) in enumerate(STANDS):
        x, z = x0 + 1 + n % 5 * 3, z0 + 2 + n // 5 * 4
        if block == 'head_snow':
            ops.append(cmd(f'/setblock {x} {y0 + 1} {z} powder_snow', False))
        elif block:
            ops.append(cmd(f'/fill {x} {y0} {z} {x} {y0 + (block == "powder_snow")} {z} {block}', False))
        sep = ',' if nbt else ''
        ops.append(cmd(f'/summon armor_stand {x}.5 {y0} {z}.5 {{UUID:[I;43,0,0,{n}]{sep}{nbt}}}', False))
    burning = []
    for n, (nbt, cells) in enumerate(BURNING):
        x, z = x0 + 1 + n * 3, z0 + 14
        for dy in cells:
            ops.append(cmd(f'/setblock {x} {y0 + dy} {z} powder_snow', False))
        ops.append(cmd(f'/summon armor_stand {x}.5 {y0} {z}.5 {{UUID:[I;43,1,0,{n}],{nbt}}}', False))
        burning.append((n, x, z))
    for step in (1, 1, 18):
        ops.append({'op': 'tick', 'n': step})
        for n, x, z in burning:
            ops.append(cmd(f'/data get entity 0000002b-0000-0001-0000-{n:012x} Fire'))
            ops.append(cmd(f'/execute if block {x} {y0} {z} powder_snow'))
            ops.append(cmd(f'/execute if block {x} {y0 + 1} {z} powder_snow'))
        for n in range(len(STANDS)):
            ops.append(cmd(f'/data get entity 0000002b-0000-0000-0000-{n:012x} TicksFrozen'))
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=16,dz=16]',
                       f'/fill {x0} {y0 - 2} {z0} {x0 + 15} {y0 + 4} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
