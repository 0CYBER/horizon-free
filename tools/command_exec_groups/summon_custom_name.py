"""`summon custom name`: `/summon`'s feedback names any entity by its CustomName."""

GROUP = 'summon custom name'

# Kinds Horizon spawns without a command target of their own, and two that have one (a mob, an item).
KINDS = ['evoker_fangs', 'snowball', 'arrow', 'trident', 'small_fireball', 'tnt', 'falling_block', 'eye_of_ender',
         'firework_rocket', 'area_effect_cloud', 'experience_orb', 'end_crystal', 'armor_stand', 'boat', 'minecart',
         'lightning_bolt', 'pig', 'item']


def cases(cmd, setup):
    """`SummonCommand.spawnEntity` answers `commands.summon.success` with `Entity.getDisplayName`: a CustomName
    loaded from the NBT argument, plain or styled, names the entity whatever its kind. Each kind is summoned
    named and unnamed, NoGravity so nothing moves, then killed."""
    x0, y0, z0 = 7008, 250, 7008
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': [[x0 >> 4, z0 >> 4]]},
           cmd('/tick freeze', False)]
    for n, kind in enumerate(KINDS):
        x, z = x0 + 1 + n % 7 * 2, z0 + 2 + n // 7 * 5
        extra = 'Item:{id:"stone",count:1},' if kind == 'item' else ''
        ops.append(cmd(f'/summon {kind} {x}.5 {y0} {z}.5 {{{extra}NoGravity:1b,CustomName:"n{n}"}}'))
        ops.append(cmd(f'/summon {kind} {x}.5 {y0} {z + 2}.5 {{{extra}NoGravity:1b,'
                       f'CustomName:{{text:"s{n}",color:"red",bold:true}}}}'))
        ops.append(cmd(f'/summon {kind} {x}.5 {y0} {z + 1}.5 {{{extra}NoGravity:1b}}'))
    # Names `ComponentSerialization.CODEC` refuses (a number, a byte, an empty list) and edge cases (an empty
    # string, an empty compound, a style without contents) on evoker fangs, which have no command target.
    for n, bad in enumerate(['5', '1b', '[]', '""', '{}', '{color:"red"}', '["a","b"]', '{translate:"entity.minecraft.pig"}']):
        ops.append(cmd(f'/summon evoker_fangs {x0 + 2 + n}.5 {y0} {z0 + 14}.5 {{CustomName:{bad}}}'))
    return ops + setup('/tick unfreeze', f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=16,dz=16]',
                       f'/fill {x0} {y0 - 2} {z0} {x0 + 15} {y0 + 4} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
