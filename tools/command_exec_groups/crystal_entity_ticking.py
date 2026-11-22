"""`crystal entity ticking`: an end crystal ticks only in an entity-ticking chunk (#18626)."""

GROUP = 'crystal entity ticking'


def cases(cmd, setup):
    """`ServerLevel.tick` ticks an entity only where its chunk is entity-ticking. `EndCrystal.tick` runs
    `applyEffectsFromBlocks`, which resets the loaded `Fire` of the fire-immune crystal to 0. A crystal in the
    forced chunk ticks and clears its `Fire`; one in the next chunk east, loaded around the forced one but not
    entity-ticking, keeps it. Frozen, then read tick by tick."""
    x0, y0, z0 = 6496, 250, 6496
    # Entities tick once every chunk within 2 columns is loaded (ChunkMap.prepareEntityTickingChunk).
    chunk = [[(x0 >> 4) + dx, (z0 >> 4) + dz] for dx in range(-2, 3) for dz in range(-2, 3)]
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk},
           cmd('/tick freeze', False)]
    # (x of the crystal, its Fire): two in the forced chunk, two in the chunk east of it.
    lanes = [(x0 + 8, 40), (x0 + 12, -5), (x0 + 24, 40), (x0 + 28, -5)]
    uuids = []
    for n, (x, fire) in enumerate(lanes, 1):
        uuids.append(f'{18626:08x}-0000-0000-0000-{n:04x}00000001')
        ops.append(cmd(f'/summon end_crystal {x}.5 {y0} {z0 + 8}.5 '
                       f'{{UUID:[I;18626,0,{n},1],Fire:{fire}s,ShowBottom:0b}}', expect_result=1))

    def reads():
        for uuid in uuids:
            ops.append(cmd(f'/data get entity {uuid} Fire'))

    reads()
    for _ in range(3):
        ops.extend([cmd('/tick unfreeze', False), {'op': 'tick', 'n': 1}, cmd('/tick freeze', False)])
        reads()
    ops.append(cmd('/tick unfreeze', False))
    return ops + setup(f'/kill @e[type=end_crystal,x={x0},y={y0 - 8},z={z0},dx=32,dy=16,dz=16]',
                       f'/forceload remove {x0} {z0}')
