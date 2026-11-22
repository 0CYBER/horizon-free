"""`orb invulnerable burn`: an `Invulnerable` experience orb keeps its health while it burns or sits in a cactus."""

GROUP = 'orb invulnerable burn'

# (summon NBT) for orbs far enough apart not to merge.
ORBS = [
    'Fire:100s,Invulnerable:1b,NoGravity:1b,Value:3s',  # refuses on_fire: health stays 5
    'Fire:100s,NoGravity:1b,Value:4s',  # burns: one off each 20th tick
    'Fire:45s,Invulnerable:1b,NoGravity:1b,Value:5s,Health:2s',
]


# Orbs inside a cactus (on sand): (summon NBT).
CACTUS = [
    'Invulnerable:1b,NoGravity:1b,Value:6s',  # refuses cactus: health stays 5
    'NoGravity:1b,Value:7s',  # one off a tick: gone after five
]


def cases(cmd, setup):
    """`Entity.baseTick`'s fire step hurts a burning orb for 1 on each 20th tick through
    `ExperienceOrb.hurtServer`, whose `isInvulnerableToBase` refuses `on_fire` for an `Invulnerable`
    orb: its health stays and the fire counts down as before. `CactusBlock.entityInside` hurts through
    the same `hurtServer`, which refuses `cactus` too. Read after 1, 20 and 40 ticks."""
    x0, y0, z0 = 7776, 250, 7776
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': [[x0 >> 4, z0 >> 4]]}]
    for n, nbt in enumerate(ORBS):
        ops.append(cmd(f'/summon experience_orb {x0 + 2 + n * 5}.5 {y0} {z0 + 4}.5 {{UUID:[I;46,0,0,{n}],{nbt}}}',
                       False))
    for n, nbt in enumerate(CACTUS):
        x, z = x0 + 2 + n * 5, z0 + 10
        ops.append(cmd(f'/setblock {x} {y0 - 1} {z} sand', False))
        ops.append(cmd(f'/setblock {x} {y0} {z} cactus', False))
        ops.append(cmd(f'/summon experience_orb {x}.5 {y0}.25 {z}.5 {{UUID:[I;46,1,0,{n}],{nbt}}}', False))
    for step in (1, 1, 2, 16, 20):
        ops.append({'op': 'tick', 'n': step})
        for n in range(len(CACTUS)):
            ops.append(cmd(f'/data get entity 0000002e-0000-0001-0000-{n:012x} Health'))
    for step in (1, 19, 20):
        ops.append({'op': 'tick', 'n': step})
        for n in range(len(ORBS)):
            for field in ('Health', 'Fire'):
                ops.append(cmd(f'/data get entity 0000002e-0000-0000-0000-{n:012x} {field}'))
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=16,dz=16]',
                       f'/fill {x0} {y0 - 2} {z0} {x0 + 15} {y0 + 2} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
