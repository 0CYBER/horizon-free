"""`passenger saved pos` (#16090)."""

GROUP = 'passenger saved pos'


def cases(cmd, setup):
    """#16090: `Entity.saveWithoutId` saves a passenger's `Pos` at its vehicle's x and z with its own y,
    in `/data get entity` and in the vehicle's `Passengers`: right after a /ride, before the vehicle seats it,
    and for the two pigs of a boat, whose seats are off the boat's centre."""
    pig, cow, stand = '@e[tag=psp_r,limit=1]', '@e[tag=psp_v,limit=1]', '@e[tag=psp_s,limit=1]'
    pigs = ['@e[tag=psp_a,limit=1]', '@e[tag=psp_b,limit=1]']
    boat = '@e[tag=psp_boat,limit=1]'
    mob = 'NoAI:1b,NoGravity:1b,PersistenceRequired:1b'
    return [
        *setup('/fill 2 219 18 14 219 22 minecraft:stone',
               f'/summon minecraft:cow 4.5 220 20.5 {{{mob},Tags:["psp_v"]}}',
               f'/summon minecraft:pig 6.0 220 20.5 {{{mob},Tags:["psp_r"]}}',
               '/summon minecraft:armor_stand 12.5 220 20.5 {NoGravity:1b,Tags:["psp_s"]}',
               f'/summon minecraft:pig 11.0 220 19.0 {{{mob},Tags:["psp_t"]}}'),
        cmd(f'/ride {pig} mount {cow}'),
        cmd(f'/ride @e[tag=psp_t,limit=1] mount {stand}'),
        cmd(f'/data get entity {pig} Pos'),
        cmd(f'/data get entity {cow} Passengers[0].Pos'),
        cmd('/data get entity @e[tag=psp_t,limit=1] Pos'),
        *setup(f'/summon minecraft:oak_boat 8.5 220 20.5 {{Rotation:[30.0f,0.0f],Tags:["psp_boat"],Passengers:['
               f'{{id:"minecraft:pig",{mob},Tags:["psp_a"]}},{{id:"minecraft:pig",{mob},Tags:["psp_b"]}}]}}'),
        {'op': 'tick', 'n': 2},
        cmd(f'/data get entity {pig} Pos'),
        cmd('/data get entity @e[tag=psp_t,limit=1] Pos'),
        *[cmd(f'/data get entity {p} Pos') for p in pigs],
        cmd(f'/data get entity {boat} Passengers[0].Pos'),
        cmd(f'/data get entity {boat} Passengers[1].Pos'),
        *setup('/kill @e[tag=psp_r]', '/kill @e[tag=psp_v]', '/kill @e[tag=psp_s]', '/kill @e[tag=psp_t]',
               '/kill @e[tag=psp_a]', '/kill @e[tag=psp_b]', '/kill @e[tag=psp_boat]', '/kill @e[type=item]',
               '/fill 2 219 18 14 219 22 minecraft:air'),
    ]
