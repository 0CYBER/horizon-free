"""`mob powder snow freezing` (#16100)."""

GROUP = 'mob powder snow freezing'
TAG_FREEZE = 15992  # UUIDs of mob_powder_snow_freezing's mobs


def cases(cmd, setup):
    """`LivingEntity.aiStep` melts `TicksFrozen` by 2 outside powder snow, and powder snow's `FREEZE` effect
    (`InsideBlockEffectType`, run from `applyEffectsFromBlocks`) adds 1 a tick inside it: a mob in powder snow
    freezes one tick a tick, once. A stray is in `#freeze_immune_entity_types` and stays at 0."""
    x0, y0, z0 = 20, 236, 24
    kinds = ['pig', 'villager', 'zombie', 'stray']
    ops = []
    for n, kind in enumerate(kinds, 1):
        x = x0 + 3 * n
        ops += setup(f'/setblock {x} {y0} {z0} minecraft:powder_snow')
        ops.append(cmd(f'/summon {kind} {x}.5 {y0} {z0}.5 {{UUID:[I;{TAG_FREEZE},0,{n},1],NoAI:1b,NoGravity:1b,'
                       f'PersistenceRequired:1b}}', expect_result=1))

    def reads():
        return [cmd(f'/data get entity {TAG_FREEZE:08x}-0000-0000-0000-{n:04x}00000001 TicksFrozen')
                for n in range(1, len(kinds) + 1)]

    for _ in range(3):
        ops += [{'op': 'tick', 'n': 1}, *reads()]
    ops += setup(f'/fill {x0} {y0} {z0} {x0 + 3 * len(kinds)} {y0} {z0} minecraft:air')
    for _ in range(2):
        ops += [{'op': 'tick', 'n': 1}, *reads()]
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 4},z={z0 - 4},dx=16,dy=8,dz=8]')
