"""`wind charge rotation` (#16089)."""

GROUP = 'wind charge rotation'
TAG_WIND = 16089  # UUIDs of wind_charge_rotation's charges


def cases(cmd, setup):
    """`AbstractWindCharge.tick` runs `AbstractHurtingProjectile.tick`, which turns the charge with
    `ProjectileUtil.rotateTowardsMovement(this, 0.2F)` before it moves, as it turns fireballs
    (#14466): the yaw and pitch go a fifth of the way towards the movement's each tick, not all
    the way at once."""
    x0, y0, z0 = 20, 240, 28
    # (type, extra NBT): sideways, a climb from a set rotation, a yaw that wraps (170 to -170).
    cases = [('wind_charge', 'Motion:[0.6d,0.0d,0.0d]'),
             ('breeze_wind_charge', 'Motion:[0.0d,0.0d,0.6d]'),
             ('wind_charge', 'Rotation:[30.0f,-20.0f],Motion:[0.2d,0.3d,0.4d]'),
             ('breeze_wind_charge', 'Rotation:[170.0f,0.0f],Motion:[-0.0868241d,0.0d,-0.9848078d]')]
    ops = []
    for n, (kind, extra) in enumerate(cases, 1):
        ops.append(cmd(f'/summon {kind} {x0 + 4 * n}.5 {y0} {z0}.5 {{UUID:[I;{TAG_WIND},0,{n},1],{extra}}}',
                       expect_result=1))
    for _ in range(3):
        ops.append({'op': 'tick', 'n': 1})
        ops += [cmd(f'/data get entity {TAG_WIND:08x}-0000-0000-0000-{n:04x}00000001 {key}')
                for n in range(1, len(cases) + 1) for key in ('Rotation', 'Pos')]
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 16},z={z0 - 16},dx=32,dy=32,dz=32]')
