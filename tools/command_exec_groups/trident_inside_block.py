"""`trident inside block`: a trident whose position is inside a collision box stops (#18287)."""

GROUP = 'trident inside block'


def cases(cmd, setup):
    """`AbstractArrow.tick` opens by reading the block at `blockPosition()`: with physics on, a position inside
    one of its collision boxes zeroes the motion and sets `inGround`. Inside stone and the lower half of a bottom
    slab it stops; above the slab, in the same block but outside its shape, and in grass, which has no collision
    shape, it flies on. Frozen, then read tick by tick."""
    x0, y0, z0 = 6368, 250, 6368
    # Entities tick once every chunk within 2 columns is loaded (ChunkMap.prepareEntityTickingChunk).
    chunk = [[(x0 >> 4) + dx, (z0 >> 4) + dz] for dx in range(-2, 3) for dz in range(-2, 3)]
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk},
           cmd('/tick freeze', False)]
    # (block, height of the trident in the block)
    lanes = [('stone', 0.5), ('smooth_stone_slab[type=bottom]', 0.25), ('smooth_stone_slab[type=bottom]', 0.75),
             ('short_grass', 0.5)]
    uuids = []
    for n, (block, height) in enumerate(lanes, 1):
        z = z0 + 1 + (n - 1) * 3
        uuids.append(f'{18287:08x}-0000-0000-0000-{n:04x}00000001')
        ops.append(cmd(f'/setblock {x0 + 4} {y0 - 1} {z} stone', expect_result=1))
        ops.append(cmd(f'/setblock {x0 + 4} {y0} {z} {block}', expect_result=1))
        ops.append(cmd(f'/summon trident {x0 + 4}.5 {y0 + height} {z}.5 '
                       f'{{UUID:[I;18287,0,{n},1],Motion:[0.01d,0d,0d],NoGravity:1b,pickup:0b}}', expect_result=1))

    # `setInGround` starts the shake (`onSyncedDataUpdated`: not on the first tick, and not over a shake still
    # running): these fly for two ticks, then a block appears around them.
    flown = [('trident', ''), ('arrow', ''), ('trident', 'shake:3b,'), ('arrow', 'shake:3b,')]
    for n, (kind, extra) in enumerate(flown, 5):
        z = z0 + 1 + (n - 5) * 3
        uuids.append(f'{18287:08x}-0000-0000-0000-{n:04x}00000001')
        ops.append(cmd(f'/summon {kind} {x0 + 9}.5 {y0}.5 {z}.5 '
                       f'{{UUID:[I;18287,0,{n},1],{extra}Motion:[0.001d,0d,0d],NoGravity:1b,pickup:0b}}',
                       expect_result=1))

    def reads():
        for uuid in uuids:
            for key in ('Motion', 'Pos', 'inGround', 'shake'):
                ops.append(cmd(f'/data get entity {uuid} {key}'))

    def step():
        ops.extend([cmd('/tick unfreeze', False), {'op': 'tick', 'n': 1}, cmd('/tick freeze', False)])
        reads()

    reads()
    for _ in range(2):
        step()
    for n in range(len(flown)):
        ops.append(cmd(f'/setblock {x0 + 9} {y0} {z0 + 1 + n * 3} stone', expect_result=1))
    for _ in range(3):
        step()
    ops.append(cmd('/tick unfreeze', False))
    return ops + setup(f'/kill @e[type=trident,x={x0},y={y0 - 8},z={z0},dx=16,dy=24,dz=16]',
                       f'/kill @e[type=arrow,x={x0},y={y0 - 8},z={z0},dx=16,dy=24,dz=16]',
                       f'/fill {x0 + 4} {y0 - 1} {z0} {x0 + 9} {y0} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
