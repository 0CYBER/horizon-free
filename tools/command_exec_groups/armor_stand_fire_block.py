"""`armor stand fire block`: an armor stand standing in a fire block burns as vanilla's does."""

GROUP = 'armor stand fire block'

STANDS = [
    '',
    'Fire:-1s',
    'Fire:-2s',  # fireIgnite counts it up; the in_fire hurt then lights it for 5 seconds
    'Fire:-5s',
    'Fire:50s',
    'Invisible:1b',  # hurtServer refuses: fireIgnite alone
    'Invulnerable:1b',
    'NoGravity:1b',  # noPhysics: no block reaches it
    'corner',  # on the edge of two fire blocks: one FIRE_IGNITE, an in_fire hurt from each
]


def cases(cmd, setup):
    """`BaseFireBlock.entityInside` applies FIRE_IGNITE every tick (`fireIgnite`: a negative `Fire`
    counts up by one, else `igniteForSeconds(8)`, so `Fire` holds at 160 against `baseTick`'s
    countdown), then hurts with `inFire`: `ArmorStand.hurtServer` takes 0.15 off a burning stand
    (IGNITES_ARMOR_STANDS) and lights one that is not for 5 seconds. `baseTick`'s burn on each 20th
    `Fire` (`on_fire`, BURNS_ARMOR_STANDS) takes 4, so a stand held at 160 loses 4.15 a tick and
    breaks. Each stand is read for `Fire` and `Health` for six ticks, then its drop is counted."""
    x0, y0, z0 = 7904, 250, 7904
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': [[x0 >> 4, z0 >> 4]]},
           cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} stone', False)]
    for n, nbt in enumerate(STANDS):
        x, z = x0 + 1 + n % 4 * 4, z0 + 2 + n // 4 * 6
        ops.append(cmd(f'/setblock {x} {y0} {z} fire', False))
        if nbt == 'corner':
            ops.append(cmd(f'/setblock {x + 1} {y0} {z} fire', False))
            ops.append(cmd(f'/summon armor_stand {x + 1}.0 {y0} {z}.5 {{UUID:[I;47,0,0,{n}]}}', False))
            continue
        sep = ',' if nbt else ''
        ops.append(cmd(f'/summon armor_stand {x}.5 {y0} {z}.5 {{UUID:[I;47,0,0,{n}]{sep}{nbt}}}', False))
    for _ in range(6):
        ops.append({'op': 'tick', 'n': 1})
        for n in range(len(STANDS)):
            for field in ('Fire', 'Health'):
                ops.append(cmd(f'/data get entity 0000002f-0000-0000-0000-{n:012x} {field}'))
    ops.append(cmd(f'/execute if entity @e[type=item,x={x0},y={y0 - 2},z={z0},dx=16,dy=6,dz=16,'
                   f'nbt={{Item:{{id:"minecraft:armor_stand"}}}}]'))
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=16,dz=16]',
                       f'/fill {x0} {y0 - 2} {z0} {x0 + 15} {y0 + 2} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
