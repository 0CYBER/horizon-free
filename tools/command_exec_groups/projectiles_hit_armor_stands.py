"""`projectiles hit armor stands` (#16119)."""

GROUP = 'projectiles hit armor stands'
TAG_STAND = 15994  # UUIDs of projectiles_hit_armor_stands' stands (second word 0) and projectiles (1)


def cases(cmd, setup):
    """`ArmorStand.isPickable` (not a marker) makes a stand a target for every projectile. An arrow, a trident,
    a wind charge and a wither skull hurt it with a type in `#always_kills_armor_stands`, which breaks it at once.
    A small fireball without an owner hurts with `unattributed_fireball` and a snowball with `thrown`, which the
    stand refuses (the snowball still breaks on it). A stand without gravity has `noPhysics`: the arrow still hits
    it, the small fireball (`AbstractHurtingProjectile.canHitEntity`) passes through. An invisible stand
    refuses every hit (the arrow turns back), and a marker isn't pickable (the arrow flies through it)."""
    y0 = 236
    shot = 'NoGravity:1b,Motion:[1.0d,0.0d,0.0d]'
    # (stand x, stand z, stand NBT, projectile, projectile NBT): rows z 14 and 20 the plain ones, z 26 the blasts.
    lanes = [(23, 20, '', 'arrow', ',pickup:0b'), (29, 20, '', 'trident', ',pickup:0b'),
             (35, 20, '', 'snowball', ''), (41, 20, ',Invisible:1b', 'arrow', ',pickup:0b'),
             (23, 14, ',Marker:1b,NoGravity:1b', 'arrow', ',pickup:0b'), (23, 26, '', 'small_fireball', ''),
             (31, 26, '', 'wind_charge', ''), (39, 26, '', 'wither_skull', ''),
             (29, 17, ',NoGravity:1b', 'arrow', ',pickup:0b'), (35, 17, ',NoGravity:1b', 'small_fireball', '')]
    ops = [*setup(f'/fill 18 {y0 - 1} 12 44 {y0 - 1} 28 minecraft:stone')]
    for n, (x, z, extra, kind, data) in enumerate(lanes, 1):
        ops.append(cmd(f'/summon armor_stand {x}.5 {y0} {z}.5 {{UUID:[I;{TAG_STAND},0,{n},1]{extra}}}',
                       expect_result=1))
        ops.append(cmd(f'/summon {kind} {x - 1}.0 {y0 + 0.6} {z}.5 {{UUID:[I;{TAG_STAND},1,{n},1],{shot}{data}}}',
                       expect_result=1))
    # A dispensed arrow, no owner: the dispenser faces east, three blocks from the stand.
    n = len(lanes) + 1
    ops += setup(f'/setblock 32 {y0} 14 minecraft:dispenser[facing=east]{{Items:[{{Slot:0b,id:"minecraft:arrow",'
                 'count:1}]}')
    ops.append(cmd(f'/summon armor_stand 35.5 {y0} 14.5 {{UUID:[I;{TAG_STAND},0,{n},1]}}', expect_result=1))
    ops.append({'op': 'crafter_tick', 'pos': [32, y0, 14]})

    def reads():
        out = []
        for k in range(1, n + 1):
            out.append(cmd(f'/data get entity {TAG_STAND:08x}-0000-0000-0000-{k:04x}00000001 Pos'))
            if k < n:
                out.append(cmd(f'/data get entity {TAG_STAND:08x}-0000-0001-0000-{k:04x}00000001 Motion'))
        return out

    for _ in range(6):
        ops += [{'op': 'tick', 'n': 1}, *reads()]
    return ops + setup(f'/kill @e[type=!player,x=16,y={y0 - 8},z=8,dx=32,dy=16,dz=24]',
                       f'/fill 18 {y0 - 1} 12 44 {y0} 28 minecraft:air')
