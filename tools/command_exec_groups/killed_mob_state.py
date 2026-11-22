"""`killed mob state` (#16103)."""

GROUP = 'killed mob state'
TAG_KILL = 15993  # UUIDs of killed_mob_state's mobs


def cases(cmd, setup):
    """`LivingEntity.kill` is `hurtServer` with a `generic_kill` hit of `Float.MAX_VALUE`: the mob saves
    `HurtTime` 10 and `DeathTime` 0 until its first death tick, an `Invulnerable` mob dies too and a totem
    doesn't save it (`#bypasses_invulnerability`). An `@` selector takes `Entity::isAlive` (health above 0,
    `EntitySelectorParser`), so a mob a command killed is out of `@e` at once, while its UUID still finds it."""
    x0, y0, z0 = 20, 236, 4
    mob = 'NoAI:1b,NoGravity:1b,PersistenceRequired:1b'
    cases = [('pig', ''), ('slime', ',Size:0'), ('zombie', ',Invulnerable:1b'),
             ('zombie', ',equipment:{mainhand:{id:"minecraft:totem_of_undying",count:1}}')]
    ops = []
    for n, (kind, extra) in enumerate(cases, 1):
        ops.append(cmd(f'/summon {kind} {x0 + 3 * n}.5 {y0} {z0}.5 {{UUID:[I;{TAG_KILL},0,{n},1],{mob},'
                       f'Tags:["kms{n}"]{extra}}}', expect_result=1))
    # The fifth dies by /damage instead: it leaves the selectors as it dies, too.
    ops.append(cmd(f'/summon slime {x0 + 15}.5 {y0} {z0}.5 {{UUID:[I;{TAG_KILL},0,5,1],{mob},Size:0,Tags:["kms5"]}}',
                   expect_result=1))

    def uuid(n):
        return f'{TAG_KILL:08x}-0000-0000-0000-{n:04x}00000001'

    def reads():
        return [cmd(f'/data get entity {uuid(n)} {key}') for n in range(1, 6)
                for key in ('Health', 'HurtTime', 'DeathTime')] + \
            [cmd(f'/execute if entity @e[tag=kms{n}]') for n in range(1, 6)]

    ops += [cmd(f'/kill @e[tag=kms{n}]') for n in range(1, 5)]
    ops.append(cmd(f'/damage {uuid(5)} 4 minecraft:arrow'))
    ops += reads()
    for _ in range(2):
        ops += [{'op': 'tick', 'n': 1}, *reads()]
    box = f'x={x0},y={y0 - 4},z={z0 - 4},dx=20,dy=8,dz=8'
    return ops + setup(f'/execute as @e[type=slime,{box}] run data merge entity @s {{Size:0}}',
                       f'/kill @e[type=!player,{box}]',
                       f'/kill @e[type=item,x={x0},y={y0 - 8},z={z0 - 4},dx=20,dy=12,dz=8]')
