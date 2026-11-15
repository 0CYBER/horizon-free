"""`thrown item kept`: the first group in a file of its own (#16833)."""

GROUP = 'thrown item kept'


def cases(cmd, setup):
    """#16698: `ThrowableItemProjectile` keeps the stack it was loaded with in its entity data and
    saves it back as `Item`, wherever it has flown to since. Read after summoning, after a tick of
    flight, and after an unload and reload (frozen, so only the save and the load count)."""
    x0, y0, z0 = 3904, 260, 3776
    chunk = [[x0 >> 4, z0 >> 4]]
    subjects = [('egg', 'diamond_pickaxe'), ('snowball', 'diamond_pickaxe'), ('egg', 'blue_egg'),
                ('experience_bottle', 'stick'), ('ender_pearl', 'stick'), ('egg', None)]
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk}]
    uuids = []
    for n, (kind, item) in enumerate(subjects, 1):
        uuids.append(f'0000413a-0000-0000-0000-{n:04x}00000001')
        held = f'Item:{{id:"minecraft:{item}",count:1}},' if item else ''
        ops.append(cmd(f'/summon {kind} {x0 + 2}.5 {y0 + 2 * n} {z0 + 8}.5 '
                       f'{{UUID:[I;16698,0,{n},1],{held}Motion:[0.6d,0.0d,0.0d],NoGravity:1b}}', expect_result=1))

    def reads():
        for uuid in uuids:
            ops.append(cmd(f'/data get entity {uuid} Item.id'))

    reads()
    ops.append({'op': 'tick', 'n': 1})
    reads()
    ops += [cmd('/tick freeze', False), cmd(f'/forceload remove {x0} {z0}', False), {'op': 'tick', 'n': 60},
            cmd(f'/execute unless loaded {x0} {y0} {z0}', expect_result=1),
            cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk}]
    reads()
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=32,dz=16]', '/tick unfreeze',
                       f'/forceload remove {x0} {z0}')
